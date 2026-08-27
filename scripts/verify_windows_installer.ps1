#Requires -Version 7.0
[CmdletBinding()]
param(
    [switch]$Live,
    [string]$ExecutablePath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$installScript = Join-Path $repoRoot 'app_flutter/windows/installer/install.ps1'
$uninstallScript = Join-Path $repoRoot 'app_flutter/windows/installer/uninstall.ps1'
$failures = [System.Collections.Generic.List[string]]::new()
$expectedRules = @{
    'LANChat-Private-UDP-Discovery' = @{ Protocol = 'UDP'; Port = '53317' }
    'LANChat-Private-TCP-Control' = @{ Protocol = 'TCP'; Port = '53318' }
}

function Add-Failure([string]$Message) {
    $failures.Add($Message)
}

function Parse-Script([string]$Path) {
    $tokens = $null
    $parseErrors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile(
        $Path,
        [ref]$tokens,
        [ref]$parseErrors
    )
    foreach ($parseError in $parseErrors) {
        Add-Failure "$([IO.Path]::GetFileName($Path)) parse error: $($parseError.Message)"
    }
    return $ast
}

function Get-Commands($Ast, [string]$Name) {
    return @($Ast.FindAll({
        param($node)
        $node -is [Management.Automation.Language.CommandAst] -and
            $node.GetCommandName() -eq $Name
    }, $true))
}

$installAst = Parse-Script $installScript
$uninstallAst = Parse-Script $uninstallScript
$installText = Get-Content -LiteralPath $installScript -Raw
$uninstallText = Get-Content -LiteralPath $uninstallScript -Raw

$newRuleCommands = Get-Commands $installAst 'New-NetFirewallRule'
if ($newRuleCommands.Count -ne 2) {
    Add-Failure "install.ps1 must declare exactly two firewall rules; found $($newRuleCommands.Count)."
}

$actualSignatures = [System.Collections.Generic.HashSet[string]]::new(
    [StringComparer]::OrdinalIgnoreCase
)
foreach ($command in $newRuleCommands) {
    $text = $command.Extent.Text
    foreach ($required in @(
        '-Direction\s+Inbound',
        '-Action\s+Allow',
        '-Enabled\s+True',
        '-Profile\s+Private',
        '-Program\s+\$Executable'
    )) {
        if ($text -notmatch $required) {
            Add-Failure "Firewall command lacks required option '$required': $text"
        }
    }
    if ($text -match '-Profile\s+(?:Any|Public|Domain)') {
        Add-Failure "Firewall command enables a non-Private profile: $text"
    }
    if ($text -match '(?s)-Protocol\s+(?<protocol>UDP|TCP).*?-LocalPort\s+(?<port>\d+)') {
        [void]$actualSignatures.Add("$($Matches['protocol']):$($Matches['port'])")
    } else {
        Add-Failure "Firewall command has no fixed TCP/UDP port: $text"
    }
}
foreach ($signature in @('UDP:53317', 'TCP:53318')) {
    if (-not $actualSignatures.Contains($signature)) {
        Add-Failure "Missing firewall signature: $signature"
    }
}

foreach ($ruleName in $expectedRules.Keys) {
    $escaped = [regex]::Escape("'$ruleName'")
    if ($installText -notmatch $escaped) {
        Add-Failure "install.ps1 lacks rule name $ruleName."
    }
    if ($uninstallText -notmatch $escaped) {
        Add-Failure "uninstall.ps1 lacks rule name $ruleName."
    }
}
if (@(Get-Commands $uninstallAst 'New-NetFirewallRule').Count -ne 0) {
    Add-Failure 'uninstall.ps1 must not create firewall rules.'
}
if (@(Get-Commands $uninstallAst 'Remove-NetFirewallRule').Count -ne 1) {
    Add-Failure 'uninstall.ps1 must remove rules through one shared loop command.'
}

if ($Live) {
    if (-not $IsWindows) {
        Add-Failure 'Live firewall verification is available only on Windows.'
    } elseif ([string]::IsNullOrWhiteSpace($ExecutablePath)) {
        Add-Failure '-ExecutablePath is required with -Live.'
    } elseif (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
        Add-Failure "Installed executable does not exist: $ExecutablePath"
    } else {
        $resolvedExecutable = (Resolve-Path -LiteralPath $ExecutablePath).Path
        foreach ($entry in $expectedRules.GetEnumerator()) {
            $rules = @(Get-NetFirewallRule -Name $entry.Key -ErrorAction SilentlyContinue)
            if ($rules.Count -ne 1) {
                Add-Failure "Expected one installed firewall rule named $($entry.Key); found $($rules.Count)."
                continue
            }
            $rule = $rules[0]
            if ($rule.Enabled.ToString() -ne 'True' -or
                $rule.Direction.ToString() -ne 'Inbound' -or
                $rule.Action.ToString() -ne 'Allow' -or
                $rule.Profile.ToString() -ne 'Private') {
                Add-Failure "Installed firewall rule has unsafe properties: $($entry.Key)"
            }
            $ports = @($rule | Get-NetFirewallPortFilter)
            if ($ports.Count -ne 1 -or
                $ports[0].Protocol.ToString() -ne $entry.Value.Protocol -or
                $ports[0].LocalPort.ToString() -ne $entry.Value.Port) {
                Add-Failure "Installed firewall port differs for $($entry.Key)."
            }
            $applications = @($rule | Get-NetFirewallApplicationFilter)
            if ($applications.Count -ne 1 -or
                [string]::IsNullOrWhiteSpace($applications[0].Program) -or
                -not [IO.Path]::GetFullPath($applications[0].Program).Equals(
                    $resolvedExecutable,
                    [StringComparison]::OrdinalIgnoreCase
                )) {
                Add-Failure "Installed firewall program differs for $($entry.Key)."
            }
        }
    }
}

if ($failures.Count -gt 0) {
    $message = 'Windows installer verification failed:' +
        [Environment]::NewLine + ' - ' +
        ($failures -join ([Environment]::NewLine + ' - '))
    Write-Error $message
    exit 1
}

Write-Host 'PASS Windows installer syntax and Private-profile firewall declarations.'
if ($Live) {
    Write-Host 'PASS Installed firewall rules, ports, profile, and executable path.'
} else {
    Write-Host 'Live firewall state was not requested.'
}
