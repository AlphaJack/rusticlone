# Generated from src/install/autostart.rs by cargo xtask generate
$ErrorActionPreference = 'Stop'

# Noted by chocolateyBeforeModify.ps1 for an upgrade's install script, which does not follow an uninstall.
Remove-Item (Join-Path $env:TEMP 'rusticlone-restart.txt') -ErrorAction SilentlyContinue
