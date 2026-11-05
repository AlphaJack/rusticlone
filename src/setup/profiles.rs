//! The profiles the answers make.

use toml_edit::{Array, DocumentMut, Item, Table, value};

use super::{Key, Kind, Question, RUSTIC_CONFIG_DOCS, Role, Setup};
use crate::{profile, settings, sys};

/// A table under the question it answers.
fn section(answers: Question) -> Table {
    let mut table = Table::new();
    table.decor_mut().set_prefix(format!("\n# {answers}\n"));
    table
}

/// `key` of `table`, under the question it answers.
fn answering(table: &mut Table, key: &str, answers: Question) {
    if let Some(mut key) = table.key_mut(key) {
        key.leaf_decor_mut().set_prefix(format!("# {answers}\n"));
    }
}

/// A table that holds only tables, so has no header of its own.
fn implicit<'a>(entries: impl IntoIterator<Item = (&'a str, Item)>) -> Item {
    let mut table: Table = entries.into_iter().collect();
    table.set_implicit(true);
    Item::Table(table)
}

impl Setup {
    /// `role`'s repository, password and options, in `table`.
    fn repository(&self, role: Role, mut table: Table) -> Item {
        let (kind, fields) = (self.kind(role), self.kind(role).fields());
        let repository = self.folder(role).unwrap_or_else(|| {
            fields.iter().find(|f| f.key == Key::Repository).map_or("", |f| self.value(role, f)).to_owned()
        });
        table["repository"] = value(profile::native_drive(&format!("{}{repository}", kind.scheme())));
        table["password"] = value(self.password(role));
        // Required ones even if blank, so that the example shows what to fill in.
        let options: Table = (fields.iter())
            .filter(|f| f.required || !self.value(role, f).is_empty())
            .filter_map(|f| Some((f.key.option()?, value(self.value(role, f)))))
            .collect();
        if !options.is_empty() {
            table["options"] = Item::Table(options);
        }
        Item::Table(table)
    }

    /// `role`'s variables in `[global.env]`.
    fn env(&self, role: Role) -> impl Iterator<Item = (&'static str, Item)> {
        (self.kind(role).fields().iter())
            .filter_map(move |f| Some((f.key.env()?, self.value(role, f))).filter(|(_, v)| !v.is_empty()))
            .map(|(key, v)| (key, value(v)))
    }

    /// File names and contents, the backup profile first, then the target's if any: see [`Self::profile_name`].
    /// Their shared start names the collection.
    pub(crate) fn profiles(&self) -> Vec<(String, String)> {
        let mut primary = DocumentMut::new();
        // One entry per folder: each has its own snapshots and history, so adding a folder later leaves the
        // others' alone. The first one says what the entries answer.
        let snapshots = self.sources().into_iter().enumerate().map(|(i, source)| {
            let mut snapshot = if i == 0 { section(Question::WhatToBackUp) } else { Table::new() };
            snapshot["sources"] = value(std::iter::once(source).collect::<Array>());
            snapshot
        });
        let mut backup = Table::new();
        // Without the domain that each network gives it, which would start a new snapshot group there.
        if let Some(host) = sys::hostname().and_then(|h| h.split('.').next()).filter(|h| !h.is_empty()) {
            backup["host"] = value(host);
        }
        backup["snapshots"] = Item::ArrayOfTables(snapshots.collect());
        primary["backup"] = Item::Table(backup);
        primary["repository"] = self.repository(Role::Primary, section(Question::WhereSaved));
        let target = (self.target != Kind::None).then(|| {
            let name = self.profile_name(Role::Target);
            let mut copy = section(Question::WhereCopied);
            copy["targets"] = value(std::iter::once(name.as_str()).collect::<Array>());
            primary["copy"] = Item::Table(copy);
            let mut target = DocumentMut::new();
            target["repository"] = self.repository(Role::Target, section(Question::WhereCopied));
            let env: Table = self.env(Role::Target).collect();
            if !env.is_empty() {
                target["global"] = implicit([("env", Item::Table(env))]);
            }
            (format!("{name}.toml"), target.to_string())
        });
        if !self.keep.rules().is_empty() {
            let mut forget = section(Question::HowLongKeep);
            for (keep, n) in self.keep.rules() {
                forget[keep] = value(*n);
            }
            primary["forget"] = Item::Table(forget);
        }
        let mut vars = Table::new();
        vars.decor_mut().set_prefix("\n");
        for (setting, step, every, question) in self.schedules() {
            if let Some(cron) = self.cron(step, every) {
                vars[setting.key] = value(cron);
                answering(&mut vars, setting.key, question);
            }
        }
        // Written so that users find how much each check reads.
        if let (true, Some(subsets)) = (vars.contains_key(settings::CHECK.key), settings::CHECK_SUBSETS.default) {
            vars[settings::CHECK_SUBSETS.key] = value(subsets);
        }
        if !self.notify.trim().is_empty() {
            vars[settings::NOTIFY.key] = value(self.notify.trim());
            answering(&mut vars, settings::NOTIFY.key, Question::WhereNotify);
        }
        // Rustic sets only the backup profile's variables for a copy, and the target's for its own commands.
        vars.extend(self.env(Role::Primary).chain(self.env(Role::Target)));
        primary["global"] = implicit([("env", Item::Table(vars))]);
        let primary = format!("# {}\n{primary}", crate::i18n::t!("setup-guide", url = RUSTIC_CONFIG_DOCS));
        std::iter::once((format!("{}.toml", self.profile_name(Role::Primary)), primary)).chain(target).collect()
    }
}
