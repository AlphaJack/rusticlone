# Generated from src/install/autostart.rs by cargo xtask generate
$ErrorActionPreference = 'Stop'
$restart = Join-Path $env:TEMP 'rusticlone-restart.txt'

# Windows keeps a running executable from being replaced: rusticlone started from this package is stopped.
$tools = Split-Path -Parent $MyInvocation.MyCommand.Definition
$users = Get-CimInstance Win32_Process -Filter "Name = 'rusticlone.exe'" |
  Where-Object { $_.ExecutablePath -like "$tools\*" } |
  ForEach-Object {
    $owner = Invoke-CimMethod -InputObject $_ -MethodName GetOwner
    Stop-Process -Id $_.ProcessId -Force
    "$($owner.Domain)\$($owner.User)"
  }
if ($users) {
  $users | Sort-Object -Unique | Set-Content $restart
}
