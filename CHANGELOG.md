# Changelog

## [2.0.2](https://github.com/AlphaJack/rusticlone/compare/v2.0.1...v2.0.2) - 2026-10-28

### Fixed

- A long silent rustic call is no longer stopped as stalled on macOS and Windows ([#43](https://github.com/AlphaJack/rusticlone/pull/43))
- Hooks that print no longer break reading rustic's output ([#40](https://github.com/AlphaJack/rusticlone/pull/40))
- Stop the server on Ctrl-Break on Windows ([#36](https://github.com/AlphaJack/rusticlone/pull/36))
- Commands no longer hang on Windows while the server runs ([#35](https://github.com/AlphaJack/rusticlone/pull/35))
- Show which run holds the lock that a run waits for ([#26](https://github.com/AlphaJack/rusticlone/pull/26))

#### Backup

- A killed run leaves no rustic running on Windows ([#38](https://github.com/AlphaJack/rusticlone/pull/38))

#### Cli

- Draw notify, install and update as step trees ([#54](https://github.com/AlphaJack/rusticlone/pull/54))

#### Install

- Let package manager handle updates ([#41](https://github.com/AlphaJack/rusticlone/pull/41))
- Sign the macOS app again after an update replaces its executable ([#19](https://github.com/AlphaJack/rusticlone/pull/19))

#### Web

- Browse, search and download Windows snapshots ([#37](https://github.com/AlphaJack/rusticlone/pull/37))

### Translated

- Translate into Italian 🇮🇹 ([#25](https://github.com/AlphaJack/rusticlone/pull/25))

## [2.0.1](https://github.com/AlphaJack/rusticlone/compare/v2.0.0...v2.0.1) - 2026-10-28

### Fixed

- Build on armv7, where time_t is 32 bits ([#10](https://github.com/AlphaJack/rusticlone/pull/10))

## [2.0.0](https://github.com/AlphaJack/rusticlone/releases/tag/v2.0.0) - 2026-10-27

### Changed

- **breaking**: Rewrite in Rust as a long-running service
