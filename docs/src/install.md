# Install

## Windows

One-line installer for PowerShell:

```powershell
irm https://github.com/AlphaJack/rusticlone/releases/latest/download/rusticlone-installer.ps1 | iex; & "$HOME\.cargo\bin\rusticlone.exe" install --auto-update
```

WinGet:

```pwsh
winget install AlphaJack.Rusticlone
```

Scoop:

```pwsh
scoop bucket add alphajack https://github.com/AlphaJack/scoop-bucket
scoop install alphajack/rusticlone
```

Chocolatey:

```pwsh
choco install rusticlone
```

## macOS

One-line installer:

```sh
curl -LsSf https://github.com/AlphaJack/rusticlone/releases/latest/download/rusticlone-installer.sh | sh && $HOME/.cargo/bin/rusticlone install --auto-update
```

Homebrew:

```sh
brew install AlphaJack/tap/rusticlone
```

nix-darwin and Home Manager: see [Nix](nix.md)

## Linux

One-line installer:

```sh
curl -LsSf https://github.com/AlphaJack/rusticlone/releases/latest/download/rusticlone-installer.sh | sh && $HOME/.cargo/bin/rusticlone install --auto-update
```

Arch Linux (AUR):

```sh
yay -S rusticlone-bin
```

Homebrew:

```sh
brew install AlphaJack/tap/rusticlone
```

NixOS and Home Manager: see [Nix](nix.md)

Docker and Podman: see [Container image](container.md)

## Any OS

Cargo:

```sh
cargo binstall rusticlone
```

Manual installation: [GitHub release binaries](https://github.com/AlphaJack/rusticlone/releases)
