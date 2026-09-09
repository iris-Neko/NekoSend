param([string]$SourceRoot = (Get-Location).Path)
$ErrorActionPreference = 'Stop'
$cmake = (Get-Command cmake -ErrorAction SilentlyContinue).Source
if (-not $cmake) {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $cmake = & $vswhere -latest -products '*' -find 'Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe' | Select-Object -First 1
}
if (-not $cmake) { throw 'CMake is required' }
$build = Join-Path $SourceRoot 'app_flutter/build/windows/x64'
$previousTemp = $env:TEMP
$previousTmp = $env:TMP
try {
    # Older tagged tests compare paths literally; avoid the hosted runner's
    # RUNNER~1 TEMP alias without modifying the immutable application source.
    if ($env:GITHUB_ACTIONS -eq 'true') {
        if (-not $env:RUNNER_TEMP) { throw 'RUNNER_TEMP is required in CI' }
        $fixtureRoot = Join-Path $env:RUNNER_TEMP 'nekosend-shell-fixtures'
        New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null
        $env:TEMP = $fixtureRoot
        $env:TMP = $fixtureRoot
    }
    foreach ($configuration in @('Release', 'Debug')) {
        & $cmake --build $build --config $configuration --target shell_identity_tests
        if ($LASTEXITCODE -ne 0) { throw "Native shell test build failed: $configuration" }
        & (Join-Path $build "shell-tests/$configuration/shell_identity_tests.exe")
        if ($LASTEXITCODE -ne 0) { throw "Native shell test failed: $configuration" }
    }
} finally {
    $env:TEMP = $previousTemp
    $env:TMP = $previousTmp
}
