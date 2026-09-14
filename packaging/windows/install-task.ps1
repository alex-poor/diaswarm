# A diaswarm pool member, as a Windows scheduled task.
#
#   powershell -ExecutionPolicy Bypass -File install-task.ps1
#
# A scheduled task rather than a service, because a Windows service needs a
# service wrapper and this needs no privilege at all — it reads and writes one
# folder under your own profile and holds sealed records it cannot open.
#
# Runs at logon and restarts if it stops. A machine that should carry around
# the clock wants its sleep settings checked: a sleeping PC carries nothing.

$ErrorActionPreference = 'Stop'

$exe = Join-Path $env:LOCALAPPDATA 'Programs\diaswarm\diaswarm-peer.exe'
if (-not (Test-Path $exe)) {
    Write-Error "diaswarm-peer.exe not found at $exe — copy it there first, or edit this script."
}

$action   = New-ScheduledTaskAction -Execute $exe
$trigger  = New-ScheduledTaskTrigger -AtLogOn
# Not "only on AC power", and no time limit: an always-on carrier that stops
# after three days is the failure this whole thing exists to avoid.
$settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -ExecutionTimeLimit ([TimeSpan]::Zero) `
    -RestartCount 99 -RestartInterval (New-TimeSpan -Minutes 1) `
    -StartWhenAvailable

Register-ScheduledTask -TaskName 'diaswarm-peer' `
    -Action $action -Trigger $trigger -Settings $settings `
    -Description 'diaswarm pool peer - carries sealed records it cannot read' `
    -Force | Out-Null

Start-ScheduledTask -TaskName 'diaswarm-peer'
Write-Host "Installed and started. Stop with: Unregister-ScheduledTask -TaskName diaswarm-peer"
