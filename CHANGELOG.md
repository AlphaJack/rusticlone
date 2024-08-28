<!--
// ┌───────────────────────────────────────────────────────────────┐
// │ Contents of CHANGELOG.md                                      │
// ├───────────────────────────────────────────────────────────────┘
// │
// ├──┐Changelog - toc
// │  ├──┐[1.1.1] - 2024-08-26
// │  │  └── Changed
// │  ├──┐[1.1.0] - 2024-08-20
// │  │  ├── Added
// │  │  └── Changed
// │  ├──┐[1.0.1] - 2024-05-17
// │  │  ├── Added
// │  │  └── Changed
// │  └──┐[1.0.0] - 2024-05-15
// │     └── Various
// │
// └───────────────────────────────────────────────────────────────
-->

# Changelog - Rusticlone

## [1.1.1] - 2024-08-26
### Changed

- Supporting only rustic == 0.7, requiring both rustic and rclone to be installed

## [1.1.0] - 2024-08-20
### Added

- Added rustic and rclone version check

### Changed

- Enforcing minimum rustic and rclone versions
- Supportig rclone >=1.67

## [1.0.1] - 2024-05-17
### Added

- Added support to backup individual files

### Changed

- Not parsing /etc/rustic if profiles were found in ~/.config/rustic

## [1.0.0] - 2024-05-15
### Various

- Initial commit

