param([string]$SourceRoot = (Get-Location).Path)
$ErrorActionPreference = 'Stop'
$exe = Join-Path $SourceRoot 'app_flutter/build/windows/x64/runner/Release/lan_chat.exe'
$process = Start-Process -FilePath $exe -ArgumentList '--test-composer-clipboard' -WindowStyle Hidden -Wait -PassThru
if ($process.ExitCode -ne 0) { throw "Isolated Windows clipboard test failed: $($process.ExitCode)" }
Write-Output 'PASS isolated clipboard: CF_HDROP multi-file, file-over-text priority, plain paths, PNG bitmap encoding'
