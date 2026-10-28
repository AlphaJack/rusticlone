# Generated from src/install/autostart.rs by cargo xtask generate
$ErrorActionPreference = 'Stop'

# Windows keeps a running executable from being replaced: rusticlone started from this package is stopped.
$tools = Split-Path -Parent $MyInvocation.MyCommand.Definition
Get-CimInstance Win32_Process -Filter "Name = 'rusticlone.exe'" |
  Where-Object { $_.ExecutablePath -like "$tools\*" } |
  ForEach-Object {
    Stop-Process -Id $_.ProcessId -Force
    Write-Host "Stopped rusticlone ($($_.ProcessId)) to replace it: it starts again at the next login, or from the Start menu"
  }
