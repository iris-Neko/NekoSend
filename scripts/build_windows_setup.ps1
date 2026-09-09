param(
    [Parameter(Mandatory)][string]$Version,
    [string]$SourceRoot = (Get-Location).Path
)
$ErrorActionPreference = 'Stop'
if ($Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$') { throw 'Invalid version' }
$bundle = Join-Path $SourceRoot 'app_flutter/build/windows/x64/runner/Release'
foreach ($name in @('lan_chat.exe','flutter_windows.dll','lan_chat_core.dll','data/icudtl.dat')) {
    if (-not (Test-Path -LiteralPath (Join-Path $bundle $name))) { throw "Incomplete bundle: $name" }
}
$compiler = Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6/ISCC.exe'
if (-not (Test-Path -LiteralPath $compiler)) { throw 'Inno Setup 6 is required' }
$output = Join-Path $SourceRoot 'release-output'
New-Item -ItemType Directory -Path $output -Force | Out-Null
$shellIdentityRepair = [int](Test-Path -LiteralPath (Join-Path $SourceRoot 'app_flutter/windows/runner/shell_identity.h'))
& $compiler "/DBundleDir=$bundle" "/DAppVersion=$Version" "/DOutputDir=$output" "/DShellIdentityRepair=$shellIdentityRepair" (Join-Path $PSScriptRoot '../packaging/windows/nekosend.iss')
if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
