#!/usr/bin/env bash
set -euo pipefail

root="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
manifest="$root/packaging/linux/io.github.iris_neko.NekoSend.json"
build="$(realpath -m "$root/.flatpak-build")"
if [[ "$build" != "$root/.flatpak-build" ]]; then
  echo "Refusing to clean a build directory outside this checkout" >&2
  exit 1
fi
if [[ "$(uname -m)" != x86_64 ]]; then
  echo "This manifest currently builds the x86_64 Linux application" >&2
  exit 1
fi
version="$(python3 -c 'import sys, xml.etree.ElementTree as E; print(E.parse(sys.argv[1]).find("releases/release").attrib["version"])' "$root/packaging/linux/io.github.iris_neko.NekoSend.metainfo.xml")"
mkdir -p "$root/dist"
cd "$root"
flatpak-builder --force-clean --disable-rofiles-fuse \
  --repo="$root/.flatpak-repo" --default-branch=stable "$build" "$manifest"
bundle="$root/dist/NekoSend-$version-linux-x86_64.flatpak"
flatpak build-bundle --arch=x86_64 \
  --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
  "$root/.flatpak-repo" "$bundle" io.github.iris_neko.NekoSend stable
cd "$root/dist"
sha256sum "$(basename "$bundle")" > "$(basename "$bundle").sha256"
printf 'Built %s\n' "$bundle"
