# Rusticlone

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/AlphaJack/rusticlone/HEAD/screenshots/web-dark-collections.png">
  <img alt="Web UI collections" src="https://raw.githubusercontent.com/AlphaJack/rusticlone/HEAD/screenshots/web-light-collections.png">
</picture>

Rusticlone backs up your computers and servers on a schedule, keeps copies in more than one place and warns you when something needs your attention.

- Zero lock-in: your backups are interoperable with [rustic](https://rustic.cli.rs/), [restic](https://restic.net/) and their [ecosystem](https://github.com/rubiojr/awesome-restic)
- Simple mental model: back up (snapshot) your files to a repository, copy that snapshot to other repositories, and mark old snapshots for deletion
- Lightweight and accessible: one small binary that you can run from the command line, from your desktop tray or as a web app
- Minimal configuration: add a few lines to your existing rustic profiles and you are good to go
- Quiet and reliable: it notifies you only when there are issues, not at every run

**[Read the documentation](https://alphajack.github.io/rusticlone/)**

## Install

Windows (PowerShell):

```powershell
irm https://github.com/AlphaJack/rusticlone/releases/latest/download/rusticlone-installer.ps1 | iex; & "$HOME\.cargo\bin\rusticlone.exe" install --auto-update
```

macOS and Linux:

```sh
curl -LsSf https://github.com/AlphaJack/rusticlone/releases/latest/download/rusticlone-installer.sh | sh && $HOME/.cargo/bin/rusticlone install --auto-update
```

WinGet, Scoop, Chocolatey, Homebrew, AUR, Nix and Docker: see [Install](https://alphajack.github.io/rusticlone/install.html).

## Getting started

Run `rusticlone serve --open` to create your first rustic profiles from the web UI.

**Keep a copy of your profiles away from the computer (e.g. password manager, cloud drive, private git repo), as you'll need them to restore your files in case of a disaster.**

Next: [Getting started](https://alphajack.github.io/rusticlone/getting-started.html) and [Restore](https://alphajack.github.io/rusticlone/restore.html).

## Contributions

- Something is not clear or found a bug? Please open a detailed [issue](../../issues)
- [CONTRIBUTING.md](CONTRIBUTING.md): developer guidelines to submit PRs
