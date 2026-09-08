# Automatic Releases

The `Release All Platforms` workflow builds Windows x64 ZIP, Android 13+ ARM64
APK, Linux x86_64 Flatpak, and macOS 13+ Universal DMG/ZIP from one immutable
source commit. No developer Apple account is needed; Mac artifacts remain
ad-hoc signed and not notarized.

## Publish A Version

1. Update the Cargo workspace version, Flutter pubspec version/build number,
   AppStream release metadata, and any version assertions in tests.
2. Commit and push the changes to main.
3. Create and push a matching tag, for example `git tag v0.3.1` followed by
   `git push origin v0.3.1`.

Every `v*` tag automatically runs the workflow. Version mismatches fail before
any build or release mutation. All four platforms must succeed before the
release is published. `0.x` and suffixed versions are community prereleases;
plain `1.0.0` and later are normal releases.

To repair an existing release, run the workflow from main with its existing
tag, or use `gh workflow run release.yml --ref main -f tag=v0.3.0`.
The workflow reads the original tag, never moves it, builds all platforms,
checks manifests/hashes/source identity, uploads all packages, verifies their
GitHub checksums, and updates the release notes. Failed build jobs can be
rerun through GitHub Actions without manually copying artifacts.

Packaging helpers come from the workflow revision so older tags can be
repaired. Only generated/staged AppStream metadata and Flatpak source-exclusion
lists may be adjusted for old tags; application code is built from the tag.

## Android Signing

Run `pwsh -File scripts/setup_android_signing.ps1` once on the maintainer's
Windows host. The helper creates or reuses a private RSA signing key outside
the checkout, provisions four GitHub repository secrets and the public
`ANDROID_SIGNING_CERT_SHA256` variable. It does not rotate an existing key.

Back up the entire private signing directory offline. `recovery.json` contains
passwords: never commit, upload as an artifact, or attach it to an issue. The
directory ACL permits only its owner and SYSTEM. Losing the key prevents
normal updates to already-installed release APKs. The runner's decoded key is
kept in its temporary directory and removed after signing; no key is published.

Earlier local debug APKs use a different signing identity and cannot be
overwritten by release-signed APKs. Do not uninstall a debug build without
preserving its local history. The workflow never uses the debug-release bypass.

## Scope Of Verification

Automated checks cover shared Rust/Flutter tests, Android JVM tests, APK
signature/native library verification, full Windows bundles, Flatpak packaging,
Mac Universal architecture/signature/image integrity and Release startup.
These do not replace real-device LAN transfer, permission, notification,
clipboard, sleep and manufacturer battery-policy testing. Release notes state
these limits and invite community reports without private data.
