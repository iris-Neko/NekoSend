param([string]$SourceRoot = (Get-Location).Path)
$ErrorActionPreference = 'Stop'
$cmake = (Get-Command cmake -ErrorAction SilentlyContinue).Source
if (-not $cmake) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $cmake = & $vswhere -latest -products '*' -find 'Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe' | Select-Object -First 1
}
if (-not $cmake) { throw 'CMake is required' }
$build = Join-Path $SourceRoot 'app_flutter/build/windows/x64'
foreach ($configuration in @('Release', 'Debug')) {
    & $cmake --build $build --config $configuration --target shell_identity_tests
    if ($LASTEXITCODE -ne 0) { throw "Native shell test build failed: $configuration" }
    & (Join-Path $build "shell-tests/$configuration/shell_identity_tests.exe")
    if ($LASTEXITCODE -ne 0) { throw "Native shell test failed: $configuration" }
}
