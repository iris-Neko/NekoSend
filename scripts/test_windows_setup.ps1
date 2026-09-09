param([string]$SourceRoot = (Get-Location).Path)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Run installation lifecycle tests only on a disposable CI runner' }
$setup = Join-Path $SourceRoot 'release-output/NekoSend-windows-x64-setup.exe'
$install = Join-Path $env:LOCALAPPDATA 'Programs/LAN Chat'
$data = Join-Path $env:LOCALAPPDATA 'LAN Chat'
if (Test-Path -LiteralPath $install) { throw 'Refusing to modify an existing installation' }
New-Item -ItemType Directory -Path $data -Force | Out-Null
$sentinel = Join-Path $data 'installer-preserve-test.txt'
$database = Join-Path $data 'lan_chat.db'
$hadDatabase = Test-Path -LiteralPath $database
[IO.File]::WriteAllText($sentinel, 'Keep existing user data')
foreach ($iteration in 1..2) {
    $process = Start-Process -FilePath $setup -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /SP- /TASKS=desktopicon' -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Install/upgrade failed: $($process.ExitCode)" }
    foreach ($file in @('lan_chat.exe','flutter_windows.dll','lan_chat_core.dll','data/icudtl.dat','unins000.exe')) {
        if (-not (Test-Path -LiteralPath (Join-Path $install $file))) { throw "Missing installed file: $file" }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $env:APPDATA 'Microsoft/Windows/Start Menu/Programs/NekoSend.lnk'))) { throw 'Missing Start menu shortcut' }
    $shell = New-Object -ComObject Shell.Application
    $folder = $shell.Namespace([string](Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'))
    $shortcut = $folder.ParseName('NekoSend.lnk')
    if ($shortcut.ExtendedProperty('System.AppUserModel.ID') -ne 'dev.lanchat.LANChat') { throw 'Installer shortcut has the wrong taskbar identity' }
    $wsh = New-Object -ComObject WScript.Shell
    $link = $wsh.CreateShortcut((Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\NekoSend.lnk'))
    if ($link.IconLocation -ne "$(Join-Path $install 'lan_chat.exe'),0") { throw 'Installer shortcut has the wrong taskbar icon' }
    $desktop = [Environment]::GetFolderPath('Desktop')
    $desktopItem = $shell.Namespace([string]$desktop).ParseName('NekoSend.lnk')
    if ($desktopItem.ExtendedProperty('System.AppUserModel.ID') -ne 'dev.lanchat.LANChat') { throw 'Desktop shortcut has the wrong taskbar identity' }
    if (-not $hadDatabase -and (Test-Path -LiteralPath $database)) { throw 'Shell identity repair unexpectedly started the chat core' }
    if (-not (Test-Path -LiteralPath 'HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/{760AA855-DB24-4DF3-8D75-45573FC01949}_is1')) { throw 'Missing uninstall registration' }
}
$process = Start-Process -FilePath (Join-Path $install 'unins000.exe') -ArgumentList '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART' -WindowStyle Hidden -Wait -PassThru
if ($process.ExitCode -ne 0) { throw 'Uninstall failed' }
if (Test-Path -LiteralPath (Join-Path $install 'lan_chat.exe')) { throw 'Uninstall left the application executable behind' }
if ([IO.File]::ReadAllText($sentinel) -ne 'Keep existing user data') { throw 'Uninstall modified user data' }
Write-Output 'PASS installer: install, shortcuts, upgrade, uninstall registration, removal and preserved user data'
