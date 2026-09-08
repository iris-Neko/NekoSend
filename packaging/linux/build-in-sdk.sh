#!/usr/bin/env bash
set -euo pipefail

root="$(pwd -P)"
mkdir -p "$HOME" "$PUB_CACHE" "$CARGO_HOME"
export PATH="$root/.flutter-sdk/bin:/app/lib/nekosend-build/home/.cargo/bin:/usr/lib/sdk/llvm21/bin:$PATH"
git config --global --add safe.directory "$root/.flutter-sdk"
flutter --disable-analytics
flutter config --enable-linux-desktop
flutter precache --linux
cd "$root/app_flutter"
flutter pub get --enforce-lockfile
flutter build linux --release --no-pub

install -d /app/lib/nekosend
cp -a build/linux/x64/release/bundle/. /app/lib/nekosend/
cd "$root"
install -Dm755 packaging/linux/nekosend /app/bin/nekosend
install -Dm644 packaging/linux/io.github.iris_neko.NekoSend.desktop \
  /app/share/applications/io.github.iris_neko.NekoSend.desktop
install -Dm644 packaging/linux/io.github.iris_neko.NekoSend.metainfo.xml \
  /app/share/metainfo/io.github.iris_neko.NekoSend.metainfo.xml
install -Dm644 packaging/linux/io.github.iris_neko.NekoSend.service \
  /app/share/dbus-1/services/io.github.iris_neko.NekoSend.service
install -Dm644 packaging/linux/io.github.iris_neko.NekoSend.svg \
  /app/share/icons/hicolor/scalable/apps/io.github.iris_neko.NekoSend.svg
