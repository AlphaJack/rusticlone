# Changelog

## [2.0.4](https://github.com/AlphaJack/rusticlone/compare/v2.0.3...v2.0.4) - 2026-10-31

### Fixed

- Use dark and rounded look for the Windows tray menu ([#79](https://github.com/AlphaJack/rusticlone/pull/79))

#### Backup

- Fall back to the next repository when a restore fails ([#82](https://github.com/AlphaJack/rusticlone/pull/82))
- Say where a restore looked when no repository could be read ([#77](https://github.com/AlphaJack/rusticlone/pull/77))
- Find another computer's snapshots when restoring ([#73](https://github.com/AlphaJack/rusticlone/pull/73))
- Restore another system's folders only into a folder ([#72](https://github.com/AlphaJack/rusticlone/pull/72))
- Restore snapshots made on Windows on macOS and Linux ([#71](https://github.com/AlphaJack/rusticlone/pull/71))

#### Web

- Show the fields of the choices Firefox restores in the setup form ([#95](https://github.com/AlphaJack/rusticlone/pull/95))
- Read the backup folder in the setup form as rustic does ([#94](https://github.com/AlphaJack/rusticlone/pull/94))
- Save each setup folder as its own backup entry ([#81](https://github.com/AlphaJack/rusticlone/pull/81))
- Hide Copy and Restore when they have nothing to do ([#80](https://github.com/AlphaJack/rusticlone/pull/80))
- Refuse a backup folder inside a source or holding other files ([#75](https://github.com/AlphaJack/rusticlone/pull/75))
- Treat browsers behind a proxy or tunnel as remote ([#74](https://github.com/AlphaJack/rusticlone/pull/74))

### Performance

#### Web

- Render open pages again when what they show changes with time ([#97](https://github.com/AlphaJack/rusticlone/pull/97))

## [2.0.3](https://github.com/AlphaJack/rusticlone/compare/v2.0.2...v2.0.3) - 2026-10-29

### Fixed

#### Cli

- Clearer collections output for unusual profiles ([#66](https://github.com/AlphaJack/rusticlone/pull/66))
- List profile folders as a tree, and cover untested paths ([#65](https://github.com/AlphaJack/rusticlone/pull/65))

#### Install

- Start the macOS server at normal speed ([#61](https://github.com/AlphaJack/rusticlone/pull/61))

#### Web

- Widen the pointer target of the stop button ([#67](https://github.com/AlphaJack/rusticlone/pull/67))

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
