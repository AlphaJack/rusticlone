# Contributing

## Design

Rusticlone must stay out of the way of someone taking their first computer or server backup: easy to configure, easy to use and forget.
Rustic profiles hold most of the configuration, and Rusticlone adds few flags and environment variables of its own.
The README is the only public document for now, so it has to be a complete guide.

## Setup

```sh
cargo install cargo-binstall
rustup component add rust-analyzer # needed by cargo xtask acyclic
cargo binstall cargo-nextest cargo-insta rustic-rs cargo-deny
```

Or open the repository in its dev container (`.devcontainer`), which sets all of this up on Linux.

## Trying it out

On Linux, this runs Rusticlone on a two-profile collection:

```sh
S=$(mktemp -d)
export XDG_CONFIG_HOME=$S/.config XDG_CACHE_HOME=$S/.cache RUSTICLONE_STATE_DIR=$S/state RUSTICLONE_LISTEN=127.0.0.1:32999
mkdir -p $S/.config/rustic $S/docs
echo hello > $S/docs/a.txt
cat > $S/.config/rustic/Docs-hdd.toml <<EOF
[[backup.snapshots]]
sources = ["$S/docs"]

[repository]
repository = "$S/hdd"
password = "test"

[copy]
targets = ["Docs-cloud"]
EOF
cat > $S/.config/rustic/Docs-cloud.toml <<EOF
[repository]
repository = "$S/cloud"
password = "test"
EOF
cargo build
R=$PWD/target/debug/rusticlone
cd $S # the working directory is searched for profiles too
$R backup --all
```

## Layout

`cargo xtask acyclic --mermaid` draws the module graph.

- `rustic` drives the rustic CLI
- `request` is what can be asked for
- `result` is what users read back
- `state` is the [redb](https://www.redb.org) bookkeeping
- `live` holds the runs in progress
- `engine` runs collections
- `server` is the scheduler and the web UI

- Decision code (the engine, plans, schedules, the state's fold, parsers) does not touch files, processes, the environment or the clock. Rustic runs through `rustic::Runner`, time comes from `clock::Clock`, and the state from `state::Store`. `clippy.toml` disallows the direct calls everywhere but in the modules at the edge.
- Stored runs and the ledger must read back in every later release: the shapes in `tests/fixtures` must keep reading. Never regenerate them to make a test pass.

## Code

- The fewest lines that stay idiomatic and readable, ASCII only.
- Comments are rustdoc, or say why. None restate the code.
- Nothing is kept in sync by hand: editing one thing never means updating another file that hardcodes it.
- One type per concept. A match that maps one enum's variants one to one onto another's is a sign to merge the two, or to derive one from the other. A field that mirrors another is derived instead.
- Lookups go through the `Store` and `Ledger` helpers, not keys rebuilt by hand.
- Unused code goes, and so does code that only an earlier version needed.
- No hacks: data structures, algorithms and interactions are built for the long run. A kernel notification and SSE, not polling and a page that reloads itself.
- What should be refactored is refactored first, not built upon.

## DRY principle

- A setting is a line of `settings!` in `src/settings.rs`. `packaging/rusticlone.env` is generated from the settings read at startup and committed. `documented_variables_exist` fails on a README variable that no setting reads.
- A table of the state is a line of `tables!` in `src/state.rs`.
- Text for users is a message in `i18n/en/rusticlone.ftl`. `#[derive(Fluent)]` looks a message up by the name of the type and variant it describes, and `t!` by its id. Both fail the build on a missing message or argument, and `tests/literals.rs` fails on English in `src`. Another language is a folder beside `en`.

## Migrations

A stored or published shape (`impl shape::Versioned`) that would no longer read back gets a migration appended to its `MIGRATIONS`. A migration turns the previous version's JSON into the new one:

```rust
const MIGRATIONS: &'static [shape::Migration] =
    &[shape::Migration { up: |v| v["kept"] = v["old"].take(), read_by_previous: false }];
```

Set `read_by_previous` when the previous release already reads the new shape. A field added with `#[serde(default)]`, or one removed, reads back without a migration.

## Tools

```sh
cargo xtask generate     # rewrite the generated files: env file, Linux packages and the logo's renderings
cargo xtask fixtures     # record the installed rustic's output into tests/fixtures/rustic-MAJOR.MINOR
cargo insta review       # check changed snapshots and accept them
cargo xtask vendor       # download htmx
cargo xtask acyclic      # find dependency cycles between modules and between items
cargo xtask translations # list the messages each translation lacks, shown in English
cargo xtask screenshots  # take the README's screenshots anew, in Docker, in about 25 minutes
cargo xtask screenshots --setup              # or keep the computer they show running,
cargo xtask screenshots --only web prune     # take some of them again on it,
cargo xtask screenshots --teardown           # and remove it
```

## Checks

While working, run the tests of what changed:

```sh
cargo nextest run --lib cli::                  # one module's unit tests
cargo nextest run --test integration restore::  # one file of integration tests
cargo nextest run restores_a                    # tests by name
```

Run the `run:` steps of [ci.yml](.github/workflows/ci.yml) before pushing.
Tests for another system do not compile on yours, so the dev container covers Linux.

## Pull requests

The PR title is a conventional commit such as `feat(web): show the next run`.
PRs are squash-merged with the title as the commit and the description as its body.
[pr-title.yml](.github/workflows/pr-title.yml) checks the title against these types and scopes:

| Type | For |
| --- | --- |
| `feat` | something users can do that they could not before |
| `fix` | a bug users hit |
| `perf` | the same behaviour, faster or smaller |
| `i18n` | a new or updated translation |
| `refactor`, `docs`, `test`, `ci`, `chore` | anything users do not notice |

[release-plz.toml](release-plz.toml) gives `feat`, `fix`, `perf` and `i18n` their heading in the release notes and leaves the others out unless breaking.

The scope is optional and becomes a heading under the type. Leave it out for changes across the whole app.

| Scope | For |
| --- | --- |
| `backup` | backups, copies, restores and maintenance, through rustic and rclone |
| `web` | the dashboard and the setup form |
| `cli` | the commands and their output |
| `notify` | notifications after a run |
| `install` | installing, autostart, packages and images |
| `deps` | dependency bumps users notice such as a newer rustic |

At the end of the description, after some text:

- `Fixes #12` adds "fixes #12" to the entry.
- With `!` after the scope, `BREAKING CHANGE: <what users must do>` shows that line under the entry.

## Release process

1. Every push to main updates a release PR, in which release-plz sets the version and the changelog from the PR titles since the last release.
2. Merging it tags `vX.Y.Z` and drafts a GitHub release.
3. The tag starts [release.yml](.github/workflows/release.yml), where cargo-dist builds the binaries and installers.
4. The publish and post-announce jobs in `[workspace.metadata.dist]` of `Cargo.toml` then publish the packages. Their workflows in `.github/workflows` can be run again for a tag by hand.
