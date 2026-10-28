# Generated from src/install/autostart.rs by cargo xtask generate
$ErrorActionPreference = 'Stop'
$restart = Join-Path $env:TEMP 'rusticlone-restart.txt'

# url64bit and checksum64: set by .github/workflows/chocolatey.yml at each release.
$packageArgs = @{
  packageName    = $env:ChocolateyPackageName
  unzipLocation  = Split-Path -Parent $MyInvocation.MyCommand.Definition
  url64bit       = 'https://github.com/AlphaJack/rusticlone/releases/download/v0/rusticlone-x86_64-pc-windows-msvc.zip'
  checksum64     = 'XXXXXX'
  checksumType64 = 'sha256'
}

Install-ChocolateyZipPackage @packageArgs

# The rusticlone that chocolateyBeforeModify.ps1 stopped, started again for its users: in their session and without
# this one's elevation, as at their login. A launcher that exits starts it, as removing a running task stops it.
if (Test-Path $restart) {
  $exe = Join-Path $packageArgs.unzipLocation 'rusticlone.exe'
  foreach ($user in Get-Content $restart) {
    $task = "rusticlone-restart-$([guid]::NewGuid())"
    $launch = "Start-Process -FilePath '$exe' -ArgumentList serve -WindowStyle Hidden"
    $action = New-ScheduledTaskAction -Execute 'conhost.exe' -Argument "--headless powershell.exe -NoProfile -Command `"$launch`""
    $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    Register-ScheduledTask -TaskName $task -Action $action -Principal $principal -Settings $settings | Out-Null
    Start-ScheduledTask -TaskName $task
    $deadline = (Get-Date).AddSeconds(30)
    while ((Get-ScheduledTask -TaskName $task).State -eq 'Running' -and (Get-Date) -lt $deadline) { Start-Sleep 1 }
    Unregister-ScheduledTask -TaskName $task -Confirm:$false
    Write-Host "Started rusticlone again for $user"
  }
  Remove-Item $restart
}
