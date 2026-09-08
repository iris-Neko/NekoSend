# NekoSend 0.2.0

## Changes

- Linux x86_64 desktop client packaged as a Flatpak using GNOME 50.
- GTK/desktop-portal file selection, notifications, single-instance activation,
  clipboard integration, background window behavior and KDE StatusNotifier tray.
- Android system back navigates within the app before sending its task to the
  background; it no longer leaves the app immediately from a chat.
- Windows Enter sends; Shift+Enter inserts a newline. IME composition is not sent.
- Each device can set its own name. Android prefers a readable model name and
  preserves later custom names. Existing private chat titles follow peer renames.
- Schema V4 adds a distinct Linux platform while preserving existing identities,
  bindings, messages and pending deliveries.
- Three-platform build workflows and packaging checks.

## Downloads

- `NekoSend-0.2.0-linux-x86_64.flatpak`: install with
  `flatpak install --user ./NekoSend-0.2.0-linux-x86_64.flatpak`.
  Start with `flatpak run io.github.iris_neko.NekoSend`.
- `NekoSend-0.2.0-windows-x64.zip`: extract the complete directory and run
  `lan_chat.exe`; keep its DLLs and data directory beside it.
- `SHA256SUMS`: hashes of the downloadable application packages.

All clients must be 0.2.0 or newer to recognize Linux peers. Schema V4 databases
cannot be opened by older clients; do not downgrade against an upgraded database.
The Flatpak is distributed here, not listed on Flathub. The runtime comes from
Flathub. This release does not alter the repository's visibility.

## Verification

- Windows and Linux Rust workspace: 90 tests passed on each host.
- Flutter: 58 tests passed, including Android back navigation and naming.
- Android JVM: 7 tests passed. Android 14 instrumentation: 15 passed, one optional
  SFTP benchmark skipped. Android 16 instrumentation was not run because its
  separate test APK installation was not confirmed.
- Installed Linux Release Flatpak launched on KDE Wayland, registered its tray,
  was discovered by Windows and Android, and exchanged real TCP messages.
- Windows/Linux 32MiB roundtrip through real KDE file selectors completed with
  identical SHA-256 hashes. Linux keyboard Enter sending was exercised in the
  Release application, not through a debug extension.

## Known Limits

- Traffic is plaintext and assumes a trusted local IPv4 network.
- Android 16 on the tested Xiaomi device can freeze the background process even
  with a foreground service. Reactivating the app restores it. Battery policies
  are not disabled by this release.
- Wayland background clipboard access depends on the compositor. Autostart
  permission depends on the desktop portal. Multiple Linux distributions and all
  desktop environments have not been certified.
- No production-signed Android APK is attached. Debug validation builds are not
  presented as production releases. Windows binaries are not code-signed.
- The remaining original V1 hardware/install/documentation acceptance matrix is
  not claimed complete by this release. See docs/08-test-plan.md.
