# Changelog - toc

## [1.5.0] - 2025-09-13
### Added

- Using forget and prune options from rustic config
- Added support for rustic == 0.10
- Exit on failure, add --version

## [1.4.0] - 2024-11-22
### Added

- Added support for environment variables
- Improved apprise error messages
- Added support for notifications via apprise

### Documentation

- Added notification documentation and screenshot
- Added coverage badge

### Tests

- Renamed from 'make tests' to 'make test'

### Various

- Merge pull request #1 from AlphaJack/notifications
- Apprise Notifications

## [1.3.0] - 2024-10-03
### Added

- Added support for rustic == 0.9

### Documentation

- Updated example profiles to rustic 0.9

### Tests

- Updated test profiles to rustic 0.9

## [1.2.1] - 2024-09-22
### Documentation

- Added known issues on windows

### Fixed

- Removing "v" when checking for compatible rustic and rclone versions
- Creating log file for parallel operations

### Tests

- Added a restore after the final backup

## [1.2.0] - 2024-08-28
### Added

- Added support for multiple sources per profile
- Added support for rustic >= 0.8
- Parsing rustic toml instead of relying on regex
- Passing all environment variables to rustic and rclone

### Changed

- Requiring python >= 3.11

### Documentation

- Removed known limitations of older Rusticlone versions

### Tests

- Added multiple sources
- Added encrypted rclone config

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

### Various

- Initial commit

