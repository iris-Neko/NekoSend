"""Validate the repository's Flatpak packaging contract without building it."""

import json
import pathlib
import re
import tomllib
import xml.etree.ElementTree as ET


def main():
    root = pathlib.Path(__file__).resolve().parents[1]
    packaging = root / "packaging" / "linux"
    manifest = json.loads((packaging / "io.github.iris_neko.NekoSend.json").read_text(encoding="utf-8"))
    app_id = "io.github.iris_neko.NekoSend"
    assert manifest["app-id"] == app_id
    assert manifest["runtime"] == "org.gnome.Platform"
    assert manifest["runtime-version"] == "50"
    allowed_permissions = {
        "--share=network", "--share=ipc", "--socket=wayland",
        "--socket=fallback-x11", "--device=dri",
        "--filesystem=xdg-download/NekoSend:create",
        "--talk-name=org.kde.StatusNotifierWatcher", "--env=GTK_USE_PORTAL=1",
    }
    assert set(manifest["finish-args"]) == allowed_permissions
    for module in manifest["modules"]:
        for source in module["sources"]:
            if "url" in source:
                assert source["url"].startswith("https://")
                assert re.fullmatch(r"[a-f0-9]{64}", source["sha256"])
    assert "/lib/nekosend-build" in manifest["cleanup"]
    source = manifest["modules"][-1]["sources"][-1]
    assert source["type"] == "dir"
    assert {".git", ".e2e-data", "target", "dist", "app_flutter/android/key.properties"} <= set(source["skip"])
    metadata = ET.parse(packaging / f"{app_id}.metainfo.xml").getroot()
    assert metadata.findtext("id") == app_id
    version = metadata.find("releases/release").attrib["version"]
    cargo = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    assert version == cargo["workspace"]["package"]["version"]
    pubspec = (root / "app_flutter" / "pubspec.yaml").read_text(encoding="utf-8")
    assert re.search(rf"(?m)^version: {re.escape(version)}\+\d+$", pubspec)
    desktop = (packaging / f"{app_id}.desktop").read_text(encoding="utf-8")
    assert "Exec=nekosend" in desktop
    assert f"Icon={app_id}" in desktop
    assert "DBusActivatable=true" in desktop
    runner = (root / "app_flutter" / "linux" / "CMakeLists.txt").read_text(encoding="utf-8")
    assert f'set(APPLICATION_ID "{app_id}")' in runner
    for file in [*packaging.glob("*.sh"), packaging / "nekosend"]:
        assert b"\r\n" not in file.read_bytes(), f"Shell script needs LF: {file}"
    print(f"PASS Flatpak {version}: IDs, locked sources, permissions, metadata and build scripts")


if __name__ == "__main__":
    main()
