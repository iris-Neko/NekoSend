# NekoSend for macOS

Community test build for macOS 12 or newer, Apple Silicon and Intel.
This build uses local ad-hoc signing. It is NOT Developer ID signed or
Apple notarized. No Apple Developer membership is required to build it.

Open the DMG and drag NekoSend into Applications before launching.
For a trusted downloaded build blocked by Gatekeeper, first try opening it,
then use System Settings > Privacy & Security > Open Anyway. Do not disable
Gatekeeper globally. Managed Macs may prohibit this override.

Allow local-network access when prompted. All peers need the macOS-aware
client (schema V6); older releases ignore the new platform identifier.
This is not an App Store sandboxed application. It uses user-selected files
and Downloads/NekoSend for receiving. Database and identity are stored under
~/Library/Application Support/NekoSend. Clipboard images use the user cache.

Closing the window can leave the app running in the menu bar. Quit from its
menu to stop the core cleanly. Start at login requires macOS 13 or newer and
may require approval in Login Items. Sleep still suspends network activity.

The package build checks both CPU architectures and its ad-hoc signature.
Physical Mac LAN interoperability, permission dialogs, notifications, and
login-at-startup still need a real-device acceptance test.

Database upgrades preserve history but are one-way: do not open an upgraded
database with an older release. Back up the data directory before upgrading.
