# Folders

## Configuration file

`rusticlone install` creates it. Rusticlone reads it at start: restart Rusticlone after editing it.

| OS | regular user | root / SYSTEM |
|---|---|---|
| Linux | `$HOME/.config/rusticlone/rusticlone.env` | `/etc/rusticlone/rusticlone.env` |
| macOS | `$HOME/Library/Application Support/rusticlone/rusticlone.env` | `/etc/rusticlone/rusticlone.env` |
| Windows | `%APPDATA%\rusticlone\config\rusticlone.env` | `%PROGRAMDATA%\rusticlone\config\rusticlone.env` |

## Profiles

Rusticlone looks for profiles in the same folders as rustic:

| OS | regular user | root / SYSTEM |
|---|---|---|
| Linux | `$HOME/.config/rustic` | `/root/.config/rustic`<br>`/etc/rustic` |
| macOS | `$HOME/Library/Application Support/rustic` | `/var/root/Library/Application Support/rustic`<br>`/etc/rustic` |
| Windows | `%APPDATA%\rustic\config` | `%WINDIR%\System32\config\systemprofile\AppData\Roaming\rustic\config`<br>`%PROGRAMDATA%\rustic\config` |

Profiles in the working directory are also considered, so it's easy to run one-off commands without moving files.

## State

Rusticlone state is meant to be disposable, as the source of truth for your backups remains your repositories.
CLI and server both write to the same shared folder, by default:

| OS | regular user | root / SYSTEM |
|---|---|---|
| Linux | `$HOME/.local/state/rusticlone` | `/var/lib/rusticlone` |
| macOS | `$HOME/Library/Application Support/rusticlone` | `/Library/Application Support/rusticlone` |
| Windows | `%LOCALAPPDATA%\rusticlone` | `%PROGRAMDATA%\rusticlone\state` |

There you can find rustic logs, process locks and a [redb](https://www.redb.org/) database with historical run details and repository statistics.
