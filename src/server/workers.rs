//! Runs requested by the web UI and the schedule, each on its own thread. Locks coordinate what they
//! use, and their records form the queue.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

use anyhow::Result;
use jiff::Timestamp;

use super::{App, Logged, notice::Notice};
use crate::{
    clock,
    engine::{Asked, Batch, Engine},
    log::{self, Level, Message},
    request::{Request, RestoreOptions, RunKind, Selection, Trigger},
    result::Status,
    state::RunLog,
};

/// Threads running requests.
#[derive(Default)]
pub(super) struct Workers(Arc<AtomicUsize>);

/// Counts as one of the [`Workers`] until dropped.
struct Working(Arc<AtomicUsize>);

impl Workers {
    pub(super) fn busy(&self) -> bool {
        self.0.load(Ordering::Relaxed) > 0
    }

    fn start(&self) -> Working {
        self.0.fetch_add(1, Ordering::Relaxed);
        Working(self.0.clone())
    }
}

impl Drop for Working {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// What [`App::submit`] did with each request.
#[derive(Default)]
pub(super) struct Submitted {
    /// Profile, run id, and what it does ("checking primary 3/10").
    pub(super) started: Vec<(String, i64, String)>,
    /// Labels of requests already queued or running here.
    pub(super) skipped: Vec<String>,
}

impl App {
    /// Requests `kind` on `selection`, as the web UI does.
    pub(super) fn submit(
        self: &Arc<Self>,
        selection: &Selection,
        kind: RunKind,
        options: &RestoreOptions,
    ) -> Result<Submitted> {
        let engine = self.engine();
        let mut submitted = Submitted::default();
        for (collection, request) in engine.requests(selection, kind, options)? {
            match self.ask(&engine, &collection, &request, Trigger::Web)? {
                Some((id, doing)) => submitted.started.push((collection, id, doing)),
                None => submitted.skipped.push(request.label(&collection)),
            }
        }
        Ok(submitted)
    }

    /// Queues `request` and runs it on a thread, unless it joins the same request queued in any process.
    /// Returns the run's id and what it does. If the same request runs, it waits.
    pub(super) fn ask(
        self: &Arc<Self>,
        engine: &Engine,
        collection: &str,
        request: &Request,
        trigger: Trigger,
    ) -> Result<Option<(i64, String)>> {
        let Some(j) = engine.collection(collection) else { return Ok(None) };
        let Asked::Queued(queued) = engine.ask(&j, request, trigger)? else { return Ok(None) };
        let (id, doing) = (queued.record.id, queued.request.doing(collection));
        log::event(Some(j.title), &Logged::Requested { label: queued.record.label.clone(), by: trigger.who() });
        self.pages.changed_of(collection);
        let (app, collection, working) = (self.clone(), collection.to_owned(), self.workers.start());
        thread::spawn(move || {
            // The profiles may have been reloaded.
            let (engine, batch) = (app.engine(), Batch::new(trigger));
            let request = queued.request.clone();
            if let Some(j) = engine.collection(&collection) {
                let report = engine.execute(&j, queued, &batch);
                report.print();
                // Shown on whichever page is open now.
                if let Some(why) =
                    report.results.iter().find(|l| l.level() == Level::Failure).filter(|_| trigger == Trigger::Web)
                {
                    app.pages.failed(Notice::RunFailed {
                        collection: report.collection.clone(),
                        line: Box::new(why.clone()),
                    });
                }
                engine.notify(&j);
            }
            // Stopped with the server: queued again at the next start.
            if clock::interrupted()
                && trigger == Trigger::Web
                && engine.store.run(&collection, id).is_some_and(|r| r.status == Status::Interrupted)
            {
                _ = RunLog::queued(&engine.store, &collection, request, trigger);
            }
            drop(working);
            app.pages.changed_of(&collection);
            // Skipped because it runs elsewhere: what is due stays the same until that run ends.
            if batch.ran.load(Ordering::Relaxed) {
                app.wake.notify_one();
            }
        });
        Ok(Some((id, doing)))
    }

    /// Requests what is due, and returns when to check again.
    pub(super) fn ask_due(self: &Arc<Self>) -> Option<Timestamp> {
        let engine = self.engine();
        let (due, next) = engine.agenda(engine.store.now());
        for (collection, request) in due {
            // It would be skipped, and its record would wake this again: request it once that run ends.
            if engine.collection(&collection).is_some_and(|j| engine.runs_already(&j, &request)) {
                continue;
            }
            if let Err(e) = self.ask(&engine, &collection, &request, Trigger::Schedule) {
                log::event(Some(&engine.title(&collection)), &log::Logged::Error { error: format!("{e:#}") });
            }
        }
        next
    }

    /// Stops the collection's runs in this process, or only run `id`. A queued run is removed from the queue
    /// in any process. Returns how many were stopped.
    pub(super) fn cancel(&self, collection: &str, id: Option<i64>) -> Result<usize> {
        let engine = self.engine();
        let mut stopped = 0;
        for l in self.runs.of(collection).into_iter().filter(|l| id.is_none_or(|id| l.id == id)) {
            l.cancel();
            stopped += 1;
        }
        for r in engine.store.unfinished(Some(collection)) {
            if r.status == Status::Queued && id.is_none_or(|id| r.id == id) && engine.store.alive(&r) {
                stopped += usize::from(engine.store.dequeue(collection, r.id)?);
            }
        }
        if stopped > 0 {
            log::event(Some(&engine.title(collection)), &Logged::CancelledFromWeb);
            self.pages.changed_of(collection);
        }
        Ok(stopped)
    }

    /// Stops a step of a run of this process. False if the run does not run here.
    pub(super) fn cancel_step(&self, collection: &str, id: i64, step: usize) -> bool {
        let live = self.runs.of(collection).into_iter().find(|l| l.id == id);
        live.inspect(|l| l.cancel_step(step)).is_some()
    }
}
