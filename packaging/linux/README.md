# NekoSend Linux / Flatpak

The Linux desktop client uses GTK3, Flutter and the same Rust core as Windows
and Android. Its platform identity is `linux`; update the other clients to
0.2.0 or later before using them with Linux. Older clients ignore unknown
platform values. Schema V4 preserves identities, conversations and pending
work, but older clients cannot open a database after it has been upgraded.

## Install

Install the downloaded bundle with:

```sh
flatpak install --user ./NekoSend-0.2.0-linux-x86_64.flatpak
flatpak run io.github.iris_neko.NekoSend
```

The GNOME 50 runtime is installed from Flathub if needed. The application is
not submitted to Flathub; the bundle is distributed through this repository's
GitHub release. A private repository requires access to download its release.

## Build

On an x86_64 Linux machine, install Flatpak Builder and the runtime/SDK:

```sh
flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak install --user flathub org.gnome.Sdk//50 org.gnome.Platform//50 org.freedesktop.Sdk.Extension.llvm21//25.08
bash packaging/linux/build-flatpak.sh
```

The manifest pins Flutter 3.47.1 and verifies its SHA-256. The build uses
Rust 1.98.0 through Rustup and the repository's Cargo and Dart lockfiles.
Network access is needed during the build to fetch locked dependencies;
this is a local/GitHub build manifest, not an offline Flathub submission.
The toolchain is removed from the final bundle. Output is in `dist/`.

## Desktop Behavior

- The application is single-instance. Launching it again presents the window.
- Closing the window keeps the task running when the background setting is on.
  A StatusNotifier tray is available on supporting desktops such as KDE.
  On desktops without a tray, launch NekoSend again to reopen it. Ctrl+Q or
  `flatpak run io.github.iris_neko.NekoSend --quit` requests a normal exit.
- File/folder selection uses the desktop portal. The default writable receive
  folder is `~/Downloads/NekoSend` (localized XDG Downloads when configured).
  Other files and directories are accessible only when explicitly selected.
- Notifications use GApplication/desktop portals; activating a notification
  opens the corresponding conversation, including from a cold start.
- Autostart changes use the Background portal and can require desktop approval.
- Clipboard uses GTK. Wayland compositors may restrict clipboard observation
  while unfocused; do not assume Windows-style unattended sync on all desktops.
- Data is stored below
  `~/.var/app/io.github.iris_neko.NekoSend/data/NekoSend/`. Uninstalling without
  `--delete-data` keeps it. Source files selected through a portal must remain
  accessible for queued or resumed sends.

## Network and Security

The app uses UDP 53317 discovery and TCP 53318 for control and file traffic.
Allow these ports only on a trusted LAN if the host firewall blocks them.
Flatpak network permission does not override the host firewall. The installer
does not alter firewall rules automatically.

Traffic is plaintext on the local network. No accounts, cloud relay, TLS or
authentication are added by the Flatpak package. Do not use it on an untrusted
network. It does not request access to the whole home directory, the system
bus, or arbitrary host commands.
