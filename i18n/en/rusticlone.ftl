## Text for users. The code defines four terms: `{ -name }`, the command's name, `{ -app }`, the app's name,
## `{ -env }`, the prefix of its variables, and `{ -description }`, Cargo's description of the app.
## A message ends without a period, even after several sentences. A value a user gave is written as it is;
## backticks mark what to type: a command, a flag or a key. A placeholder is in lower case. Counted words stay in
## each message because other languages inflect them by sentence.

## The command line: its help, and the collections command

help-json = Log JSON lines
help-verbose = Also print the rustic log that every run saves
help-all = { label-every-collection }
help-collection = All repositories of the collection with this name or backup profile
help-dry-run = Show what the command does without doing or recording it
help-repo = The repository with this profile: a collection's primary or a copy target
help-due = Run what is due and exit, for cron: scheduled backups, pending or failed copies, scheduled prunes and checks
help-backup = Back up a collection, copy it to its copy targets and forget old snapshots
help-copy = Copy missing snapshots from the primary to the copy targets and forget old snapshots
help-prune = Prune the repositories
help-check = Check the repositories: read the next subset of their data or only their structure
help-restore = Restore the sources from --repo or from the repository with the newest snapshots
help-restore-dry-run = Show what the restore would write where. Without other options, read the snapshots and count what a restore in place creates, replaces and keeps
help-restore-at-time = The newest snapshots taken at or before TIME (2026-10-01, "2026-10-01 12:00")
help-restore-snapshot = This snapshot of the --repo repository (default: the primary)
help-restore-to = Restore into DIR. A --snapshot-path goes under its name. Each source goes under its full path
help-restore-missing = Only missing or empty paths. Never a mount point whose device is missing
help-restore-source = Only the source with this path (default: every source)
help-restore-snapshot-path = Only this path of the snapshot
help-recreate = Copy a missing repository back: the primary from its newest target, a target from the primary
help-recreate-from = Copy from this profile instead. For the primary: a copy target. For a target: the primary or another target
help-notify = Send each collection's status to its notification URLs now, to test them
help-collections = Show every collection, or the ones named: sources, repositories, schedules, last runs and issues
help-collections-refresh = List the repositories first: their snapshots, sizes and how far the copies are behind
help-serve = Start each run when it is due and serve the web UI
help-serve-open = Open the web UI in the browser and log in
help-install = { $root ->
    [true] Start the server now and with the computer
   *[false] Start the server now and at each login
}
help-install-auto-update = Also turn on automatic updates in { -name }.env: at each start, the server updates { -name } and the rustic it is tested with, after checking their checksums and GitHub attestations
help-uninstall = { $root ->
    [true] Stop the server and no longer start it with the computer
   *[false] Stop the server and no longer start it at login
}
help-uninstall-delete = Also delete the state folder: the runs and the login token. The profiles and the repositories stay
help-update = Replace this executable with a newer release, if there is one
help-update-dry-run = Only show whether a newer release exists
help-healthcheck = Exit 0 if the server answers (a container healthcheck)
cli-unselected = name what to act on: --all, --collection or --repo
cli-due = due
cli-refresh = refresh
cli-ask-keep = Copy { $files } to { $dir } so { -name } finds them from any folder?
collections-directories = profile folders: { $dirs }
collections-sources = sources
collections-repositories = repositories
collections-issues = issues
collections-reason = reason
collections-size = size
collections-files = files
collections-profile = profile
collections-location = location
collections-snapshots = snapshots
collections-status = status
collections-unknown = { repo-status-unknown }
collections-failing = FAILING { $failures }
# A schedule and when its step last ran: `@weekly, last 2 days ago`.
collections-scheduled = { $cron }, last { $ran }
collections-not-yet = { $cron }, no { $step } yet
collections-unscheduled = manual
collections-unschedulable = invalid: { $error }
# A repository's status with what a listing found: `up to date, newest snapshot 2 hours ago`.
collections-newest = { $status }, newest snapshot { $taken }
collections-behind = { $status }, { since-behind }
collections-listed = { $status } (listed { $age } ago)
collections-kind-backup-profile = backup profiles
collections-kind-copy-target = copy targets
collections-kind-unused = unused profiles (no backup profile uses their repository)
collections-kind-fragment = fragment profiles
collections-kind-broken = broken profiles
collections-warning-hidden = { $read } hides { $hidden }: rustic reads only the first file with a profile's name. Rename or delete { $hidden }

## Runs and steps: what they do and what they did

run-kind-label = { $kind ->
    [backup] backup
    [copy] copy
    [prune] prune
    [check] check
    [restore] restore
    [test-restore] restore (dry-run)
   *[re-create] re-create
}
run-kind-doing = { $kind ->
    [backup] backing up
    [copy] copying
    [prune] pruning
    [check] checking
    [restore] restoring
    [test-restore] restoring (dry-run)
   *[re-create] re-creating
}
trigger-who = { $trigger ->
    [cli] a command
    [schedule] the schedule
   *[web] the web UI
}
request-scope-copy-to = to { $target }
request-scope-copy-to-all = to every target
request-scope-from = from { $repo }
request-scope-collection = collection
step-snapshot = snapshot to { $repo }
step-copy = copy to { $to }
step-forget = forget on { $repo }
step-prune = prune { $repo }
step-check = check { $repo } { $reading }
step-recreate = re-create { $repo }
step-recreate-from = re-create { $repo } from { $from }
step-forgetting = forgetting
status-queued = queued
status-running = running
status-succeeded = succeeded
status-succeeded-with-warnings = succeeded with warnings
status-failed = failed
status-skipped = skipped
status-cancelled = cancelled
status-interrupted = interrupted
reading-what = { $kind ->
    [structure] its structure
    [subset] data subset { $reading }
    [all] all its data
   *[sample] { $reading } of its data at random
}
blocked-no-targets = no copy targets
blocked-primary-missing = the primary is missing: if its disk is unplugged, connect it; if it was deleted, re-create it
doing-copy-from = from { $from }
doing-waiting-for-listing = waiting for { $repo } to list what was written
doing-waiting-for = waiting for { $by }
doing-listing = listing { $repo }
result-backed-up = { $count ->
    [0] backed up, nothing changed
   *[other] backed up { $ids }
}
result-backed-up-new = backed up { $ids } into the new repository
result-copied = copied { $snapshots ->
    [one] 1 snapshot
   *[other] { $snapshots } snapshots
}
result-in-sync = { repo-status-in-sync }
result-forgot = { $snapshots ->
    [0] nothing to forget
    [one] forgot 1 snapshot
   *[other] forgot { $snapshots } snapshots
}
result-forgot-held = kept { $held ->
    [one] 1 expired snapshot
   *[other] { $held } expired snapshots
} until copied
result-forgot-shrunk = kept { $shrunk ->
    [one] 1 expired snapshot
   *[other] { $shrunk } expired snapshots
} because the newest one is almost empty
result-pruned = pruned
result-pruned-freed = pruned { $removed }, { $remaining } left ({ $unused } unused)
result-verified = checked { $what }
result-would-back-up = would back up { $files ->
    [one] 1 file
   *[other] { $files } files
}, { $size }
result-would-copy = would get { $snapshots ->
    [one] 1 snapshot
   *[other] { $snapshots } snapshots
}
result-would-copy-created = would be created and get { $snapshots ->
    [one] 1 snapshot
   *[other] { $snapshots } snapshots
}
result-would-forget = { $snapshots ->
    [0] nothing to forget
    [one] would forget 1 snapshot
   *[other] would forget { $snapshots } snapshots
}
result-would-prune = would prune
result-would-prune-freed = would prune { $removed } and leave { $remaining } ({ $unused } unused)
result-would-check = would check { $what }
result-would-recreate = would be re-created
result-would-create-primary = its first backup would create it
result-would-run = would run: { $run }
result-nothing-due = nothing due until { $run } at { $at }
result-nothing-scheduled = nothing due or scheduled
result-primary-created = created with the chunker of { $from }
result-recreated = re-created from { $from }, { $snapshots ->
    [one] 1 snapshot
   *[other] { $snapshots } snapshots
}
result-target-recreated = re-created
result-listable = can be listed, so not re-created
result-primary-recreated-empty = re-created empty: no copy target has its snapshots
result-test-restored = { $repo }/{ $snapshot }: read { $size }
# `keep`: files that already match the snapshot. `additional`: files there that the snapshot lacks.
result-test-restored-plan = { $path }: would create { $create ->
    [one] 1 file
   *[other] { $create } files
}, replace { $replace }, keep { $keep } and leave { $additional } that are not in the snapshot
result-test-restored-no-counts = { $path }: rustic reported no counts
result-restored = restored from { $repo } { $snapshot } ({ $time })
result-restored-to = restored from { $repo } { $snapshot } ({ $time }) to { $to }
result-would-restore = would restore { $files ->
    [one] 1 file
   *[other] { $files } files
}, { $size }, from { $repo } { $snapshot }
result-would-restore-to = would restore { $files ->
    [one] 1 file
   *[other] { $files } files
}, { $size }, from { $repo } { $snapshot } to { $to }
result-backup-restored = the backup goes on from the snapshots of { $from }
result-left-as-it-is = left as it is: not empty
result-running-already = already running, skipped
result-joined = joined the { $label } that { $who } started in process { $pid }: its results are above
result-held-back = { $paths } is missing or empty, but the repository has snapshots of it from this computer: restore them first with `{ -name } restore`, or back up anyway with `{ -name } backup`
result-skipped = the last backup could not read { $count ->
    [one] 1 item
   *[other] { $count } items
}: { $items }
result-skipped-more = the last backup could not read { $count } items, among them { $items }
result-skipped-item = { $path } ({ $error })
result-parent-unreadable = the backup read every file again: the previous snapshot has no data for { $path } ({ $error }). Run `{ -name } check`
result-shrunk = { $paths } shrank from { $before } to { $now }: check that nothing was deleted by mistake. Older snapshots are kept until a new one reaches 1% of { $before }
result-prune-postponed = prune postponed until the copies to it are done
result-keeps-everything = skipped: the repository keeps every snapshot because it has no [forget] policy
result-unavailable = unavailable: { $error }
result-some-sources = only { $found } of { $of } sources have snapshots: back up first to restore the others
result-unmounted = not restored: its mount point has no device. Mount it and restore again, or restore to a folder
result-no-place = not restored in place: that place is not on this computer. Restore it to a folder
result-hooked = not restored in place: a restore does not run the backup hooks that can mount it. Restore it to a folder
result-not-recreated = not re-created from { $from }: { $error }
result-unmet = skipped because { $needs } did not succeed
result-no-target-listed = no copy target can be listed
result-missing-skipped = { $run } { $repo } skipped: { issue-missing }
result-unusable = { $broken ->
    [true] copy target { $target } cannot be read: see the error of its profile
   *[false] copy target { $target } has no profile: create { $target }.toml, or remove it from [copy] targets
}

## Rustic: its output, and what it did not do

rustic-untested = rustic { $found } is newer than the tested { $tested }: { -name } can misread its output. Update { -name }, or install rustic { $tested }
# `ended`: how a failed command ended, `exit 1`. `said`: the error it reported, if any.
rustic-failure = { $has_ended ->
    [true] rustic { $command } failed ({ $ended })
   *[false] rustic { $command } exited 0 without doing it
}{ $has_said ->
    [true] : { $said }
   *[false] ; see rustic's log
}
ended-exit = exit { $code }
ended-killed = killed by a signal
change-identical = identical
change-added = added
change-removed = removed
change-changed = changed
change-metadata = metadata changed
change-type = type changed
# Rustic's cause when it cannot resolve a source's path, as a user reads it.
output-unresolved = it does not exist or cannot be reached
# Why a file that another program holds open was left out: Windows says it in the user's language, without what to do.
output-in-use = another program has it open and locked. Close that program before the backup

## A collection's health

unconfigured-new = No collection found: open Rusticlone to create your first one
unconfigured-gone = No collection found: place your profiles back to the folder or create a new one 
level-word = { $level ->
    [success] healthy
    [warning] degraded
   *[failure] failing
}
# What to do about a repository that cannot be listed, after its name.
fix-missing-repo = if its disk is unplugged, connect it; if it was deleted, re-create it with `{ -name } recreate --repo { $repo }`
issue-prunes-on-forget = { $repo } prunes at every forget: slow and unscheduled. Remove `prune = true` from [forget] and set { $key } instead
issue-setting-ignored = { $repo } { $setting }
issue-never-backed-up = never backed up
issue-no-disk-access = macOS does not let rustic read { $source }. In System Settings, give Full Disk Access to { $exe }, or to the terminal that runs it
issue-missing = { $repo } is missing: { fix-missing-repo }
issue-stale = { $repo } is overdue
issue-undelivered = { notification-undelivered }
issue-since = { $issue }: { $since }. Check that the server runs
since-behind = { $age } behind
since-last-backup = last backup { $age } ago
last-never = not yet
repo-status-missing = missing
repo-status-copy-failing = copy failing
repo-status-not-created = not created
repo-status-not-checked = copies not compared
repo-status-stale = overdue
repo-status-not-backed-up = not backed up
repo-status-behind = copy pending
repo-status-in-sync = up to date
repo-status-unknown = unknown

## Notifications

notification-not-recorded = cannot record the notification: { $error }
notification-undelivered = notification failed: { $error }
notification-warned = notification sent with a warning: { $warning }
notification-notified =
    notification sent:
    { $text }
notification-would-notify =
    notification not sent (dry run):
    { $text }
notification-invalid-urls = invalid notification URL: { $urls }
notification-no-urls = no notification URLs: set { $key }
# The title: `nas: Docs is now failing`.
notification-title = { $host }: { $collection } is now { level-word }
notification-test =
    { $name } test notification
    { $emoji } It works!
# A recovery's first line: `🟢 Was failing for 2 days`.
notification-lasted = { $emoji } Was { level-word } for { $age }
# After a recovery: `🔴 2 other collections failing`.
notification-others = { $emoji } { $count ->
    [one] 1 other collection
   *[other] { $count } other collections
} { level-word }

## The web UI: headings, column names and form labels

heading-collections = Collections
heading-collection = Collection
heading-collection-name = Collection name
heading-repositories = Repositories
heading-repository = Repository
heading-sources = Sources
heading-source = Source
heading-folders = Folders
heading-snapshots = Snapshots
heading-snapshot = Snapshot
heading-snapshot-path = Snapshot path
heading-issues = Issues
heading-runs = Runs
heading-recent-runs = Recent runs
heading-run-details = Run details
heading-run = Run
heading-steps = Steps
heading-step = Step
heading-commands = Commands
heading-command = Command
heading-log = Log
heading-not-found = Not found
heading-name = Name
heading-size = Size
heading-modified = Modified
heading-actions = Actions
heading-kind = Kind
heading-path = Path
heading-paths = Paths
heading-files = Files
heading-added = Added
heading-previous = Previous
heading-current = Current
heading-file-info = Mode, owner, size, modified
heading-backed-up = Backed up
heading-checked = Checked
heading-pruned = Pruned
heading-status = Status
heading-time = Time
heading-host = Host
heading-index = #
heading-result = Result
heading-duration = Duration
heading-start = Start
heading-wait = Wait
heading-trigger = Trigger
heading-waiting-for = Waiting for
heading-using = Using
heading-from = From
heading-to = To
heading-at-time = At time
heading-dry-run = Dry run
heading-token = Token
heading-desktop = Desktop
heading-changes-in = Changes in { $snapshot }
heading-find-in = Find in { $repo }
heading-set-up = Set up a backup
heading-profiles-saved = Rustic profiles saved
heading-restore-on-new-computer = To restore on a new computer

## The web UI: buttons and links

button-back-to-collections = Back to collections
button-back-up = Back up
button-back-up-all = Back up all
button-check-all = Check all
button-prune-all = Prune all
button-refresh-all = Refresh all
button-refresh = Refresh
button-copy = Copy
button-copy-here = Copy here
button-copy-this = Copy { $text }
button-check = Check
button-prune = Prune
button-recreate = Re-create
button-recreate-this = Re-create { $repo }
button-create = Create
button-restore = Restore
button-download = Download
button-download-previous = Download as it was
button-download-current = Download as it is
button-dismiss = Dismiss
button-test-notification = Send test notification
button-stop = Stop
button-cancel = Cancel
button-changes = Changes
button-find = Find
button-find-a-file = Find a file
button-copy-commands = Copy commands
button-copy-log = Copy log
button-log-in = Log in
button-log-out = Log out
button-save = Save
button-add-folder = Add a folder
button-remove-folder = Remove this folder
button-open-folder = { $os ->
    [macos] Show in Finder
    [windows] Show in File Explorer
   *[other] Open the folder
}
button-copy-folder-path = Copy folder path
button-see-how = See how
button-cron-syntax = Cron syntax

## The web UI: words and states, in the tables and the breadcrumbs

label-every-collection = Every collection
label-unknown = { repo-status-unknown }
label-error = error
label-live = live
label-connecting = connecting...
label-reconnecting = reconnecting...
label-waiting = waiting
label-stopping = stopping
label-waiting-for-the-running = waiting for the running one
label-left = unfinished: its process stopped
label-elsewhere = running in another process, started by { $who }
label-already-queued = { $what } already queued
label-listing = listing its snapshots
label-listing-now = listing
label-since = Since
label-latest = the newest snapshot of all repositories
label-snapshot-word = snapshot
label-changes-crumb = changes
label-find-crumb = find
# The run a schedule starts, before its schedule: `backup 0 3 * * *, next tomorrow`.
label-backup-schedule = backup
label-copy-schedule = copy after the backup
label-check-schedule = check
label-prune-schedule = prune
label-not-scheduled = { $what } not scheduled
label-unschedulable = { $what } not scheduled: { $error }
label-scheduled = { $what } { $cron }
label-scheduled-next = { $what } { $cron }, next { $next }

## The web UI: sentences, tooltips and placeholders

hint-copied = Copied
hint-cannot-copy = Cannot copy here
hint-log-selected = Log selected: press Ctrl+C
hint-source-code = Source code
hint-empty = Empty
hint-nothing-changed = Nothing changed
hint-no-runs = No runs yet
hint-wrong-token = Wrong token
hint-nothing-to-restore = No snapshot to restore
hint-size-on-disk = On disk: compressed, deduplicated
hint-size-of-files = Of the files
hint-size-backed-up = Of the files as last backed up
hint-files-backed-up = As last backed up
hint-added-on-disk = New data on disk, compressed
hint-size-outdated = Measured before the last change. Refresh to update
hint-holder = The run that holds what this one waits for
hint-first-snapshot = The first snapshot of its sources in this repository: everything in it is new
hint-found-nowhere = Not in any snapshot of { $collection } here
hint-more-holding = { $more ->
    [one] and 1 more snapshot with the same files
   *[other] and { $more } more snapshots with the same files
}
hint-no-snapshot-here = No snapshot of { $collection } here
hint-create-profiles-in = Answer a few questions, or write your rustic profiles in { $dir }
hint-write-profiles-in = Write your rustic profiles in { $dir }: the web UI cannot save them there
hint-keep-a-copy = Keep a copy of these profiles away from this computer, for example in a password manager: you need them to restore after a disaster
hint-now-visible = Your collection is now in { $app }. Put the profiles of other collections in the same folder
hint-profiles-step = Put your profiles in { $dir } or another folder
hint-install-step = Install { $app }:
hint-restore-step = Restore every collection:
hint-resume-step = Then start the backup server again:
hint-token-in = The token is in { $file } on { $host }. Or run `{ -name } serve --open` there to log in without it
hint-token-set = The token is set in { $key } on { $host }. Or run `{ -name } serve --open` there to log in without it
hint-only-if-unlisted = Runs only if the repository cannot be listed, and never changes an existing one
hint-created-empty = { result-primary-recreated-empty }
hint-newest-target = the copy target with the newest snapshots
hint-from-primary = the primary: re-created empty, its snapshots follow with the next copy
hint-log-in-first = log in first
hint-metrics-only = the metrics token opens { $metrics } and { $status } only: log in with the admin token
hint-cross-site = refused: another site sent it. Open { -name } directly and try again
hint-name-or-path = part of a name or path
hint-where-it-was = where it was, or a folder such as /tmp/restored
hint-now = now, or a time such as 2026-10-01 12:00
hint-notify = notification URLs, or nothing
notice-started = { $collection }: { $what }
notice-refreshing = { $collection }: refreshing
notice-refreshing-repo = { $collection }: refreshing { $repo }
notice-stopping = { $collection }: stopping
notice-dismissed = { $collection }: dismissed
notice-notified = { $collection }: notification sent
notice-not-notified = { $collection }: { notification-undelivered }
notice-test-notified = test notification sent
notice-test-not-notified = test notification failed: { $error }
notice-folder-not-opened = the folder did not open: the server's log says why
notice-run-failed = { $collection }: { $line }
notice-destination-not-absolute = the destination must be an absolute path
notice-not-submitted = { $collection }: { $error }
notice-setup-not-saved = not saved: { $error }
notice-not-running-here = { $collection }: not running here
notice-nothing-to-dismiss = { $collection }: nothing to dismiss
notice-cannot-stop = cannot stop: { $error }
notice-cannot-dismiss = cannot dismiss: { $error }
missing-page = page not found
missing-asset = asset not found
missing-run-kind = run kind not found
missing-collection = collection not found
missing-run = run not found
missing-repository = repository not found
missing-snapshot = snapshot not found
free-missing = folder or disk not found
free-bytes = { $size } free

## The metrics' help

metric-process-start-time-seconds = when the server started
metric-build-info = always 1, with the version as a label
metric-collection-running = 1 while the collection has a run in progress
metric-issues = the collection's issues of this kind
metric-newest-snapshot-timestamp-seconds = the newest snapshot in the repository
metric-behind-since-timestamp-seconds = the oldest snapshot that a copy target does not have
metric-snapshots = the collection's snapshots in the repository
metric-stored-bytes = the repository's packed data
metric-snapshot-files = the files in the newest snapshot of each source
metric-snapshot-processed-bytes = the bytes the newest snapshot of each source read
metric-snapshot-added-bytes = the packed data that the newest snapshot of each source added
metric-snapshot-duration-seconds = the time the newest snapshot of each source took
metric-last-success-timestamp-seconds = the step's last success
metric-consecutive-failures = the step's failures since its last success

## The setup form

question-what-to-back-up = What do you want to back up?
question-where-saved = Where do you want to keep the backup?
question-where-copied = Where do you want a copy of the backup?
question-when-back-up = How often do you want to back up?
question-how-long-keep = How long do you want to keep backups?
question-how-often-check = How often do you want to check the backup for damage?
question-where-notify = Where do you want notifications about issues?
first-run-kind-none = Nowhere
first-run-kind-local = External drive
first-run-kind-smb = SMB share
first-run-kind-nfs = NFS share
first-run-kind-s3 = S3-compatible
first-run-kind-sftp = SFTP
first-run-kind-webdav = WebDAV
first-run-kind-rest = REST server
first-run-kind-gdrive = Google Drive
first-run-kind-onedrive = OneDrive
first-run-kind-dropbox = Dropbox
first-run-kind-pcloud = pCloud
first-run-kind-rclone = Other
group-mounted = Local
group-direct = OpenDAL services
group-rclone = Rclone services
every-manual = Manually
every-hourly = Hourly
every-daily = Daily at midnight
every-weekly = Weekly on Sunday
every-monthly = Monthly on the 1st
every-custom = Custom
keep-year = One year
keep-month = One month
keep-all = Everything
first-run-key-repository = repository
field-repository-password = Repository password
field-server-key = Server key
field-choice-accept = Any key
field-choice-add = Remember first key
field-choice-strict = Known keys only
field-rclone-password = Rclone config password
field-rclone-password-command = Or a command that prints the rclone config password
field-rclone-hint = Configure the remote with rclone first
field-remote = Remote
field-folder = Folder
field-path = Path
field-server = Server
field-user = User
field-private-key = Private key file
field-region = Region
field-bucket = Bucket
field-access-key = Access key
field-secret-key = Secret key
field-password = Password
field-email = Email
field-url = URL
first-run-guide = full configuration guide: { $url }
first-run-no-choice = no choice { $choice }
first-run-error-name = the name must start with a letter or a digit; the other characters can be letters, digits, `-`, `_` or `.`
first-run-error-relative = { $path } is not a full path: start it at the root of a disk, or at ~ for your home folder
first-run-error-no-sources = no folder to back up
first-run-error-no-primary = no primary location
first-run-error-no-custom = no custom schedule
first-run-error-incomplete = the { $role ->
    [primary] primary
   *[target] target
} location needs its { $field }
first-run-error-not-a-choice = the { $role ->
    [primary] primary
   *[target] target
} location's { $field } must be one of: { $choices }
first-run-error-no-dir = no profile folder that can be written in
first-run-error-exists = { $path } already exists

## The server, its installation and updates

server-requested = { $label } requested by { $by }
server-cancelled-from-web = cancelled from the web UI
server-serving = web UI on { $url }. Log in with { $key }
server-serving-opened = web UI on { $url }, opened in the browser
server-serving-saved-token = web UI on { $url }. Log in with the token in { $file } or start with --open
server-healthy = the server answers on { $listen }
server-not-opened = cannot open it on the desktop: { $error }
server-reloaded = profiles reloaded{ $changed }
server-not-reloaded = cannot reload profiles: { $error }
server-not-watching = cannot watch files ({ $error }): reload the server after you edit profiles
server-not-watching-dir = cannot watch { $dir }: { $error }
server-dir-not-created = cannot create { $dir }: { $error }
server-web-ui-failed = web UI: { $error }
server-no-wall-clock = no wall clock timer ({ $error }): schedules can be late after a suspend
server-runs-unread = { $runs ->
    [one] 1 run
   *[other] { $runs } runs
} not shown: written in a format this version cannot read
fault-cannot-listen = cannot listen on { $listen }. Stop the program that uses it, or set { $key } to another address
fault-restarting = restarting into the updated { -name }
fault-no-session = cannot generate a session
fault-no-token = cannot generate the login token
fault-session-not-kept = cannot save the session
fault-no-directory = no folder { $path } in the snapshot
fault-cannot-run-rustic = cannot run rustic
fault-not-piped = { rustic-error-not-piped }
fault-stopping = the server is stopping
fault-dump-failed = rustic dump failed
fault-no-longer-configured = no longer configured
health-error-not-answering = no server answers on { $listen }. Start it with `{ -name } serve`
health-error-unhealthy = the server answered { $answer }
autostart-installed = the server starts now and with the computer: { $file }
autostart-installed-at-login = the server starts now and at login: { $file }
autostart-uninstalled = the server is stopped and no longer starts automatically: { $file } deleted
autostart-uninstalled-running = the server no longer starts at login: { $file } deleted. The running one stops at logout
autostart-auto-update = the server updates itself at each start: { $file } sets { $key }=true
autostart-auto-update-managed = auto-update stays off: the package manager updates { -name }
autostart-app-refreshed = the app now runs { -name } { $version }, which the package manager installed
autostart-not-installed = the server was not installed
autostart-deleted = { $dir } deleted
autostart-error-cannot-run = cannot run { $program }
autostart-error-failed = { $command } failed: { $error }
autostart-error-no-profile-dir = no profile folder known on this computer
autostart-error-unsupported = no service manager known on this computer. Start `{ -name } serve` from your own service or cron
autostart-blocked = macOS blocks the server at login. Open System Settings > General > Login Items & Extensions and switch { $app } on
autostart-unit-description = { -app } server
# What desktops and the Windows start menu say of the app. Translations replace it.
app-description = { -description }
tray-quit = Quit (pause automatic backups)
switch-at-boot = Start at boot
switch-at-login = Start at login
switch-auto-update = Update automatically
update-current = { -name } { $current } is up to date
update-available = { -name } { $to } is available, this is { $from }
update-updated = { -name } updated from { $from } to { $to }
update-packaged = { -name } { $to } is available, this is { $from }: { $how }
update-with-homebrew = update it with `brew upgrade { -name }`
update-with-scoop = quit it from the tray and update it with `scoop update { -name }`
update-with-chocolatey = update it as administrator with `choco upgrade { -name }`
update-with-winget = update it with `winget upgrade { $id }`
update-with-nix = update the flake input or the package in your Nix configuration
update-with-container = pull the image again with `{ $tool } pull { $image }`
update-with-system = update it with your package manager or with `sudo { -name } update`
update-not-updated = not updated: { $error }
update-error-checksum = { $file } does not match its checksum: download it again
update-error-no-manifest = release { $tag } has no { $manifest }
update-error-no-archive = release { $tag } has no archive for { $target } or a compatible target
release-error-no-release = no rustic release for this computer
release-error-unsigned = the signature of { $archive } does not match the key of rustic's maintainers. Do not use it. Download rustic again from its releases page
release-ask-download = rustic is not installed. Download rustic { $version } from { $releases } into { $dir }?
release-ask-replace = Replace rustic { $found } in { $dir } with { $version } from { $releases }?
place-copied = copied { $from } to { $to }: you can delete { $from }
place-error-write = cannot write { $path }
place-error-replace = cannot replace { $path }
install-on-path = { $dir } is on PATH in new terminals: { $changed }
download-error-download = cannot download { $url }. Check the network, then try again
download-error-status = cannot download { $url }: HTTP { $status }. Try again later
download-error-no-executable = { $archive } has no { $name }
attest-error-missing = { $archive } has no build attestation
attest-error-unattested = { $workflow } did not build { $archive }: { $error }

## Settings

settings-error-no-schedule = { $expr } is not a schedule: use @daily, @weekly, or 5 fields (minute hour day month weekday) such as `0 3 * * 0`
settings-error-profile-only = { $key } is read from profiles only: put it in their [global.env], or in a profile they share with use-profiles
settings-error-env-file = cannot read { $path }
settings-error-no-config-dir = no configuration folder known on this computer
setting-admin-token = Token for the web UI
setting-auto-update = Update { -name } and the rustic at each restart
setting-backup-cron = Backup schedule (cron expression, @daily...)
setting-check-cron = Check schedule of the repository
setting-check-subsets = Data each check reads. N reads the next of N subsets (`--read-data-subset n/N`). `5%`, `2GiB` or `all` reads that much at random. 0 reads the structure only
setting-listen = Web UI address
setting-metrics-token = Token for Prometheus and other monitors, to send as `Authorization: Bearer`
setting-notify-urls = Notification URLs (Mercurio) for when a collection's status changes
setting-prune-cron = Prune schedule of the repository
setting-rustic-bin = Location of the rustic executable
setting-stall-timeout = Time to wait before stopping an unresponsive rustic command
setting-state-dir = Folder for the database and resource locks
settings-ignored = { $key } ignored: { $why }
unused-unknown = unknown setting
unused-environment-only = read from the environment only
unused-follows-profiles = a copy target takes it from its backup profiles ({ $profiles })

## Errors and the log

log-error = error: { $error }
log-state-afresh = { $path }: { $error }. { -name } starts with an empty state
log-state-unread = cannot read the state: { $error }
log-state-not-kept = cannot save the state: { $error }
log-listing-not-kept = cannot save the listing of { $repo }: { $error }
log-run-not-saved = cannot save the run: { $error }
log-not-measured = cannot measure { $repo }: { $error }
log-not-scheduled = cannot compute what is due: { $error }
log-no-profiles = no profile with backup sources found in { $dirs }. Example profiles:{ $examples }
log-not-copied = not copied: { $error }
log-note-left-out = [... { $lines } lines left out ...]
log-note-cut = ... (the first { $size } of the log are in the file)
engine-error-no-collection = { $name } is not a collection. The collections are: { $known }
engine-error-no-collections = no collections yet. Run `{ -name } serve --open` to create one. Or add rustic profiles with [[backup.snapshots]] to one of these folders: { $dirs }
engine-error-not-a-source = { $path } is not a source of the collections named
engine-error-no-repository = { $name } is not a repository of any collection
engine-error-not-a-repository = { $name } is not a repository of { $collection }
engine-error-not-of-profile = { $name } is neither { $collection } nor one of its copy targets
engine-error-backup-of-repository = a backup applies to a whole collection, not to one repository
engine-error-no-subset = no data subset was taken for the check of { $repo }
engine-error-no-subsets = { $key }: { $value } is not a number of subsets, a percentage, a size or `all`
engine-error-from-itself = cannot re-create { $repo } from itself
engine-error-not-backed-up = cannot back up { $sources }
engine-error-unexpected = rustic recorded snapshot { $snapshot } with paths { $paths }, but the sources resolve to { $expected }. { -name } cannot copy or restore it. Give each source an absolute path outside the others
engine-error-unlisted-after = cannot list the repository after the backup. Refresh it with `{ -name } collections --refresh`
engine-error-filtered = [snapshot-filter] in the profile hides snapshot { $snapshot }, so { -name } cannot copy or restore it: change the filter so that it matches, or remove it
engine-error-primary-unlisted = no backup: { $repo } cannot be listed ({ $error }): { fix-missing-repo }
engine-error-not-recreated = cannot re-create { $repo } from { $from }: { $error }
engine-error-folder-away = { $folder }, the folder of { $repo }, is missing: { fix-missing-repo }
engine-error-not-copied-into = cannot copy snapshots from { $from } into { $repo } ({ $error })
engine-error-not-listed = cannot list { $repo }
engine-error-not-copied-from = cannot copy from { $repo }
engine-error-unavailable = { $repo } is unavailable or deleted ({ $error }): { fix-missing-repo }
engine-error-unreachable = cannot be reached
engine-error-not-copied = { $failed } of { $of ->
    [one] 1 snapshot
   *[other] { $of } snapshots
} not copied: { $error }
engine-error-no-snapshot-of-them = { $repo } has no snapshot of these sources: back up first, or restore from another repository with --repo
engine-error-no-matching-snapshot = no matching snapshot in { $repo } or its targets: use a later --at-time, or back up first
engine-error-no-time = cannot parse time { $time }: use 2026-10-01, "2026-10-01 12:00" or RFC 3339
rustic-error-dry-run-creates = a dry run creates no repository
rustic-error-cannot-run = cannot run rustic: run { -name } in a terminal and let it download rustic, install rustic ({ $docs }), or set { $key } to an installed one
rustic-error-no-version = cannot read the rustic version from { $output }
rustic-error-too-old = rustic { $found } is too old: install { $major }.{ $minor } or newer
rustic-error-no-snapshot = cannot read snapshot { $id } of { $profile }
rustic-error-invalid-profile = profile { $name } is invalid: { $error }
rustic-error-unreadable = cannot read the output of `rustic { $command }` (rustic { $version }): install the tested rustic version
rustic-error-stalled = rustic { $command } killed: no output or disk activity for { $silent }. For a slow repository, raise { $key }
rustic-error-no-status = rustic exited without a status
rustic-error-not-piped = rustic output was not piped
rustic-error-percent = a percentage must be greater than 0 and at most 100
rustic-error-no-size = a size must be greater than 0
rustic-error-no-answer = no answer for rustic { $command }
rustic-error-not-a-snapshot-id = { $id } is not a snapshot id
request-error-not-a-step = { $step } is not a step
state-error-cannot-open = cannot open { $path }
state-error-dequeued = cancelled while waiting
state-error-stopped = { $stop } while waiting
state-error-newer = a newer { -name } wrote { $path }: update { -name }, or move { $path } away to start with an empty state
profile-error-not-found = profile { $name } not found in { $dirs }
profile-error-dry-run = profile { $name } sets dry-run = true, so its backups save nothing: remove dry-run from the profile
profile-error-slashed-drive = profile { $name }: rustic reads { $location } as a backend named after its drive. Write { $native }
profile-error-backslash-after-scheme = profile { $name }: on Windows, rustic reads { $location } as a local path because of its backslash. Write forward slashes after { $scheme }:

## Times

fmt-age-under-a-minute = less than a minute
fmt-age-long = a long time
fmt-age-ago = { $age } ago
# A number of a unit of time: `2 months`.
fmt-span = { $unit ->
    [year] { $n ->
        [one] 1 year
       *[other] { $n } years
    }
    [month] { $n ->
        [one] 1 month
       *[other] { $n } months
    }
    [day] { $n ->
        [one] 1 day
       *[other] { $n } days
    }
    [hour] { $n ->
        [one] 1 hour
       *[other] { $n } hours
    }
    [minute] { $n ->
        [one] 1 minute
       *[other] { $n } minutes
    }
   *[second] { $n ->
        [one] 1 second
       *[other] { $n } seconds
    }
}
