param(
    [string]$Repository = 'iris-Neko/NekoSend',
    [string]$SigningDirectory = (Join-Path $HOME '.nekosend/signing')
)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Run this provisioning helper on Windows.' }
$directory = [IO.Path]::GetFullPath($SigningDirectory)
if ($directory.TrimEnd('\') -in @([IO.Path]::GetPathRoot($directory).TrimEnd('\'), $HOME.TrimEnd('\'))) {
    throw 'Use a dedicated signing subdirectory, not a drive root or home directory.'
}
New-Item -ItemType Directory -Path $directory -Force | Out-Null
# Signing backups stay outside the public checkout and are readable only by
# this Windows user and SYSTEM. recovery.json contains private passwords.
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..')).TrimEnd('\') + '\'
if (($directory.TrimEnd('\') + '\').StartsWith($repoRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'The signing directory must be outside the repository.'
}
$user = [Security.Principal.WindowsIdentity]::GetCurrent().User
$acl = [Security.AccessControl.DirectorySecurity]::new()
$acl.SetOwner($user)
$acl.SetAccessRuleProtection($true, $false)
foreach ($sid in @($user, [Security.Principal.SecurityIdentifier]::new('S-1-5-18'))) {
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
        $sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
}
Set-Acl -LiteralPath $directory -AclObject $acl
$store = Join-Path $directory 'nekosend-release.jks'
$recovery = Join-Path $directory 'recovery.json'
if ((Test-Path -LiteralPath $store) -ne (Test-Path -LiteralPath $recovery)) {
    throw 'Incomplete signing backup. Restore the missing file; do not rotate the release key.'
}
if (Test-Path -LiteralPath $store) {
    $values = Get-Content -LiteralPath $recovery -Raw | ConvertFrom-Json
} else {
    $password = [Convert]::ToBase64String([Security.Cryptography.RandomNumberGenerator]::GetBytes(32))
    $values = [pscustomobject]@{ keyAlias = 'nekosend-release'; storePassword = $password; keyPassword = $password }
    [IO.File]::WriteAllText($recovery, ($values | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    $env:NEKOSEND_KEY_PASSWORD = $password
    try {
        & keytool -genkeypair -keystore $store -storetype JKS -alias $values.keyAlias `
            -keyalg RSA -keysize 3072 -validity 10000 -dname 'CN=NekoSend, OU=Open Source, O=NekoSend' `
            -storepass:env NEKOSEND_KEY_PASSWORD -keypass:env NEKOSEND_KEY_PASSWORD -noprompt
        if ($LASTEXITCODE -ne 0) { throw 'Signing key generation failed.' }
    } finally { Remove-Item Env:NEKOSEND_KEY_PASSWORD -ErrorAction SilentlyContinue }
}
$certificate = Join-Path $directory 'release-certificate.der'
$env:NEKOSEND_KEY_PASSWORD = $values.storePassword
try {
    & keytool -exportcert -keystore $store -alias $values.keyAlias -file $certificate -storepass:env NEKOSEND_KEY_PASSWORD
    if ($LASTEXITCODE -ne 0) { throw 'Could not validate/export the signing certificate.' }
} finally { Remove-Item Env:NEKOSEND_KEY_PASSWORD -ErrorAction SilentlyContinue }
$secrets = @{
    ANDROID_KEYSTORE_BASE64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($store))
    ANDROID_STORE_PASSWORD = $values.storePassword
    ANDROID_KEY_ALIAS = $values.keyAlias
    ANDROID_KEY_PASSWORD = $values.keyPassword
}
foreach ($name in $secrets.Keys) {
    $start = [Diagnostics.ProcessStartInfo]::new('gh')
    $start.UseShellExecute = $false
    $start.RedirectStandardInput = $true
    foreach ($argument in @('secret', 'set', $name, '--repo', $Repository)) { $start.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::Start($start)
    # Write, not WriteLine: a trailing newline would change keystore passwords.
    $process.StandardInput.Write([string]$secrets[$name])
    $process.StandardInput.Close()
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) { throw "Failed to configure GitHub secret $name" }
    $process.Dispose()
}
$fingerprint = (Get-FileHash -LiteralPath $certificate -Algorithm SHA256).Hash.ToLowerInvariant()
& gh variable set ANDROID_SIGNING_CERT_SHA256 --repo $Repository --body $fingerprint
if ($LASTEXITCODE -ne 0) { throw 'Failed to configure the public signing fingerprint.' }
Write-Output "Release signing configured. Private backup: $directory"
Write-Output "Public certificate SHA-256: $fingerprint"
