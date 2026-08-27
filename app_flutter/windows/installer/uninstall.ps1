#Requires -Version 5.1
[CmdletBinding()]
param(
    [switch]$RemoveUserData,
    [switch]$FirewallOnly
)

$ErrorActionPreference = 'Stop'
$ruleNames = @(
    'LANChat-Private-UDP-Discovery',
    'LANChat-Private-TCP-Control'
)

function Test-IsAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Remove-LanChatFirewallRules {
    foreach ($ruleName in $ruleNames) {
        Get-NetFirewallRule -Name $ruleName -ErrorAction SilentlyContinue |
            Remove-NetFirewallRule
    }
}

if ($FirewallOnly) {
    if (-not (Test-IsAdministrator)) {
        throw 'The firewall phase requires an elevated PowerShell process.'
    }
    Remove-LanChatFirewallRules
    exit 0
}

$installDirectory = Join-Path $env:LOCALAPPDATA 'Programs\LAN Chat'
$installedExecutable = Join-Path $installDirectory 'lan_chat.exe'
$running = Get-CimInstance Win32_Process -Filter "Name = 'lan_chat.exe'" |
    Where-Object { $_.ExecutablePath -eq $installedExecutable }
if ($running) {
    throw 'LAN Chat is running. Exit it from the tray before uninstalling.'
}

if (Test-IsAdministrator) {
    Remove-LanChatFirewallRules
} else {
    $arguments = @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass',
        '-File', "`"$PSCommandPath`"", '-FirewallOnly'
    )
    $elevated = Start-Process -FilePath 'powershell.exe' -Verb RunAs `
        -ArgumentList ($arguments -join ' ') -WindowStyle Hidden -Wait -PassThru
    if ($elevated.ExitCode -ne 0) {
        throw "Firewall cleanup failed with exit code $($elevated.ExitCode)."
    }
}
Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' `
    -Name 'LAN Chat' -ErrorAction SilentlyContinue
Remove-Item -LiteralPath (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\LAN Chat.lnk') `
    -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $installDirectory -Recurse -Force -ErrorAction SilentlyContinue

if ($RemoveUserData) {
    Remove-Item -LiteralPath (Join-Path $env:LOCALAPPDATA 'LAN Chat') `
        -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host 'LAN Chat has been uninstalled.'
if (-not $RemoveUserData) {
    Write-Host 'Chat history and settings were kept in LocalAppData.'
}
