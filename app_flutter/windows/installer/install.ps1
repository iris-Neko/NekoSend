#Requires -Version 5.1
[CmdletBinding()]
param(
    [string]$BundlePath = (Join-Path $PSScriptRoot '..\..\build\windows\x64\runner\Release'),
    [switch]$NoLaunch,
    [switch]$FirewallOnly,
    [string]$InstalledExecutable
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

function Quote-ProcessArgument([string]$Value) {
    return '"' + $Value.Replace('"', '\"') + '"'
}

function Set-LanChatFirewallRules([string]$Executable) {
    foreach ($ruleName in $ruleNames) {
        Get-NetFirewallRule -Name $ruleName -ErrorAction SilentlyContinue |
            Remove-NetFirewallRule
    }
    New-NetFirewallRule -Name $ruleNames[0] -DisplayName 'LAN Chat discovery (Private)' `
        -Group 'LAN Chat' -Direction Inbound -Action Allow -Enabled True `
        -Profile Private -Program $Executable -Protocol UDP -LocalPort 53317 |
        Out-Null
    New-NetFirewallRule -Name $ruleNames[1] -DisplayName 'LAN Chat transfer (Private)' `
        -Group 'LAN Chat' -Direction Inbound -Action Allow -Enabled True `
        -Profile Private -Program $Executable -Protocol TCP -LocalPort 53318 |
        Out-Null
}

if ($FirewallOnly) {
    if (-not (Test-IsAdministrator)) {
        throw 'The firewall phase requires an elevated PowerShell process.'
    }
    if (-not (Test-Path -LiteralPath $InstalledExecutable -PathType Leaf)) {
        throw "Installed executable does not exist: $InstalledExecutable"
    }
    Set-LanChatFirewallRules $InstalledExecutable
    exit 0
}

$resolvedBundle = (Resolve-Path -LiteralPath $BundlePath).Path
$sourceExecutable = Join-Path $resolvedBundle 'lan_chat.exe'
if (-not (Test-Path -LiteralPath $sourceExecutable -PathType Leaf)) {
    throw "Windows Release bundle is incomplete: $sourceExecutable"
}

$installDirectory = Join-Path $env:LOCALAPPDATA 'Programs\LAN Chat'
$installedExecutable = Join-Path $installDirectory 'lan_chat.exe'
$running = Get-CimInstance Win32_Process -Filter "Name = 'lan_chat.exe'" |
    Where-Object { $_.ExecutablePath -eq $installedExecutable }
if ($running) {
    throw 'LAN Chat is running. Exit it from the tray before installing an update.'
}

New-Item -ItemType Directory -Path $installDirectory -Force | Out-Null
Copy-Item -Path (Join-Path $resolvedBundle '*') `
    -Destination $installDirectory -Recurse -Force

if (Test-IsAdministrator) {
    Set-LanChatFirewallRules $installedExecutable
} else {
    $arguments = @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass',
        '-File', (Quote-ProcessArgument $PSCommandPath),
        '-FirewallOnly',
        '-InstalledExecutable', (Quote-ProcessArgument $installedExecutable)
    )
    $elevated = Start-Process -FilePath 'powershell.exe' -Verb RunAs `
        -ArgumentList ($arguments -join ' ') -WindowStyle Hidden -Wait -PassThru
    if ($elevated.ExitCode -ne 0) {
        throw "Firewall configuration failed with exit code $($elevated.ExitCode)."
    }
}

if (-not $NoLaunch) {
    Start-Process -FilePath $installedExecutable
}

Write-Host "LAN Chat installed in $installDirectory"
Write-Host 'Firewall access is enabled only for Private networks.'
