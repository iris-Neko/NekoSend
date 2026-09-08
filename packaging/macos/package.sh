#!/usr/bin/env bash
set -euo pipefail
cd "${1:-$(dirname "$0")/../..}"
app="$PWD/app_flutter/build/macos/Build/Products/Release/NekoSend.app"
out="$PWD/dist/macos"
mkdir -p "$out"
test -d "$app"
# Preserve both desktop architectures, including the Rust native asset.
lipo "$app/Contents/MacOS/NekoSend" -verify_arch arm64 x86_64
rust_found=0
while IFS= read -r -d '' library; do
  lipo "$library" -verify_arch arm64 x86_64
  rust_found=1
done < <(find "$app" -type f -name '*lan_chat_core*' -print0)
test "$rust_found" = 1
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict --verbose=2 "$app"
ditto -c -k --sequesterRsrc --keepParent "$app" "$out/NekoSend-macos-universal.zip"
stage=$(mktemp -d)
ditto "$app" "$stage/NekoSend.app"
ln -s /Applications "$stage/Applications"
cp packaging/macos/README.md "$stage/README.txt"
hdiutil create -volname NekoSend -srcfolder "$stage" -ov -format UDZO "$out/NekoSend-macos-universal.dmg"
hdiutil verify "$out/NekoSend-macos-universal.dmg"
cp packaging/macos/README.md "$out/README.txt"
(cd "$out" && shasum -a 256 *.zip *.dmg > SHA256SUMS)
