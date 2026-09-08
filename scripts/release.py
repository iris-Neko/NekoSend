"""Build metadata, package collection, and all-platform GitHub release gate."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tomllib
import xml.etree.ElementTree as ET
import zipfile


PACKAGES = {
    "windows": ["NekoSend-windows-x64.zip"],
    "android": ["NekoSend-android-arm64.apk", "ANDROID-SIGNING-CERT.txt"],
    "linux": ["NekoSend-linux-x86_64.flatpak"],
    "macos": ["NekoSend-macos-universal.dmg", "NekoSend-macos-universal.zip"],
}
TAG_PATTERN = r"v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)"


def command(*args):
    return subprocess.check_output(args, text=True, encoding="utf-8").strip()


def version_for(tag):
    match = re.fullmatch(TAG_PATTERN, tag)
    if not match:
        raise ValueError("Release tag must be vMAJOR.MINOR.PATCH, optionally with a prerelease suffix")
    return match[1]


def verify_versions(cargo_text, pubspec_text, version):
    cargo = tomllib.loads(cargo_text)["workspace"]["package"]["version"]
    match = re.search(r"(?m)^version:\s*([^\s+]+)\+(\d+)\s*$", pubspec_text)
    if cargo != version or not match or match[1] != version:
        raise ValueError("Tag, Cargo workspace, and Flutter versions must match")
    return int(match[2])


def resolve(args):
    version = version_for(args.tag)
    subprocess.run(["git", "fetch", "--no-tags", "origin", f"refs/tags/{args.tag}"], check=True)
    sha = command("git", "rev-parse", "FETCH_HEAD^{commit}")
    verify_versions(command("git", "show", f"{sha}:Cargo.toml"),
                    command("git", "show", f"{sha}:app_flutter/pubspec.yaml"), version)
    values = {"tag": args.tag, "sha": sha, "version": version,
              "prerelease": str(version.startswith("0.") or "-" in version).lower()}
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.writelines(f"{key}={value}\n" for key, value in values.items())
    print(json.dumps(values))


def prepare(args):
    version = version_for(args.tag)
    if command("git", "rev-parse", "HEAD") != args.sha:
        raise ValueError("Checkout does not match the release commit")
    verify_versions(Path("Cargo.toml").read_text(encoding="utf-8"),
                    Path("app_flutter/pubspec.yaml").read_text(encoding="utf-8"), version)
    # Older immutable tags may have stale AppStream metadata. Stage metadata only;
    # the application source and the tag are never rewritten.
    path = Path("packaging/linux/io.github.iris_neko.NekoSend.metainfo.xml")
    tree = ET.parse(path)
    releases = tree.getroot().find("releases")
    if releases is None:
        raise ValueError("AppStream release metadata is missing")
    if releases[0].get("version") != version:
        releases.insert(0, ET.Element("release", version=version,
                        date=command("git", "show", "-s", "--format=%cs", args.sha)))
        tree.write(path, encoding="utf-8", xml_declaration=True)
    manifest_path = Path("packaging/linux/io.github.iris_neko.NekoSend.json")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    skip = manifest["modules"][-1]["sources"][-1]["skip"]
    for excluded in [".release-tools", "release-output", "app_flutter/macos/Flutter/ephemeral"]:
        if excluded not in skip:
            skip.append(excluded)
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def verify_android_certificate(report, expected):
    # New apksigner versions label v3 signers by SDK range instead of "#1".
    fingerprints = {value.lower() for value in re.findall(
        r"(?m)^Signer [^\r\n]* certificate SHA-256 digest: ([0-9a-fA-F]{64})\s*$", report)}
    expected = expected.lower()
    if not re.fullmatch(r"[0-9a-f]{64}", expected) or fingerprints != {expected} or "android debug" in report.lower():
        raise ValueError(f"APK release certificate mismatch: expected {expected}, found {sorted(fingerprints)}")


def collect(args):
    output = Path("release-output")
    output.mkdir(exist_ok=True)
    version = version_for(args.tag)
    platform = args.platform
    if platform == "windows":
        bundle = Path("app_flutter/build/windows/x64/runner/Release")
        for name in ["lan_chat.exe", "flutter_windows.dll", "lan_chat_core.dll", "data/icudtl.dat"]:
            if not (bundle / name).is_file():
                raise ValueError(f"Windows bundle is missing {name}")
        with zipfile.ZipFile(output / PACKAGES[platform][0], "w", zipfile.ZIP_DEFLATED) as archive:
            for path in sorted(bundle.rglob("*")):
                if path.is_file():
                    archive.write(path, path.relative_to(bundle).as_posix())
    elif platform == "android":
        apk = Path("app_flutter/build/app/outputs/flutter-apk/app-release.apk")
        with zipfile.ZipFile(apk) as archive:
            if "lib/arm64-v8a/liblan_chat_core.so" not in archive.namelist():
                raise ValueError("Android APK is missing its Rust core")
        report = Path(args.certificate_report).read_text(encoding="utf-8")
        print(report)
        verify_android_certificate(report, os.environ.get("ANDROID_SIGNING_CERT_SHA256", ""))
        shutil.copy2(apk, output / PACKAGES[platform][0])
        (output / PACKAGES[platform][1]).write_text(report + "\n", encoding="utf-8")
    elif platform == "linux":
        shutil.copy2(Path(f"dist/NekoSend-{version}-linux-x86_64.flatpak"), output / PACKAGES[platform][0])
    else:
        for name in PACKAGES[platform]:
            shutil.copy2(Path("dist/macos") / name, output / name)
    files = []
    for name in PACKAGES[platform]:
        path = output / name
        if path.stat().st_size == 0:
            raise ValueError(f"Empty package: {name}")
        files.append({"name": name, "sha256": digest(path), "size": path.stat().st_size})
    manifest = {"tag": args.tag, "sha": args.sha, "platform": platform, "files": files}
    (output / f"manifest-{platform}.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def validate_artifacts(directory, tag, sha):
    expected = {name for names in PACKAGES.values() for name in names}
    manifests = {f"manifest-{platform}.json" for platform in PACKAGES}
    if {p.name for p in directory.iterdir()} != expected | manifests:
        raise ValueError("Release must contain exactly the four platform manifests and their packages")
    for platform, names in PACKAGES.items():
        manifest = json.loads((directory / f"manifest-{platform}.json").read_text(encoding="utf-8"))
        if (manifest["tag"], manifest["sha"], manifest["platform"]) != (tag, sha, platform):
            raise ValueError("Mixed release versions or source commits")
        if sorted(item["name"] for item in manifest["files"]) != sorted(names):
            raise ValueError("Package manifest differs from expected platform assets")
        for item in manifest["files"]:
            path = directory / item["name"]
            if path.stat().st_size != item["size"] or digest(path) != item["sha256"]:
                raise ValueError(f"Artifact integrity failure: {path.name}")
    return sorted(expected)


def publish(args):
    version = version_for(args.tag)
    directory = Path(args.directory)
    names = validate_artifacts(directory, args.tag, args.sha)
    repository = os.environ["GITHUB_REPOSITORY"]
    # Recheck the tag immediately before publishing; do not attach packages to a moved tag.
    remote_sha = command("gh", "api", f"repos/{repository}/commits/{args.tag}", "--jq", ".sha")
    if remote_sha != args.sha:
        raise ValueError("Release tag moved during the build")
    checksums = directory / "SHA256SUMS"
    checksums.write_text("".join(f"{digest(directory / name)}  {name}\n" for name in names), encoding="utf-8")
    notes = f"""# NekoSend {version}

四个平台均从同一版本代码自动构建，下载安装包即可互通，无需自行编译。

## 下载
- Windows x64：`NekoSend-windows-x64.zip`，完整解压后运行 `lan_chat.exe`，保留 DLL 和 data 目录。
- Android 13+ ARM64：`NekoSend-android-arm64.apk`，使用固定发布密钥签名；证书指纹见 `ANDROID-SIGNING-CERT.txt`。
- Linux x86_64：`NekoSend-linux-x86_64.flatpak`，使用 `flatpak install --user ./NekoSend-linux-x86_64.flatpak` 安装，需要 Flathub 的 GNOME 50 运行时。
- macOS 13+ Apple Silicon / Intel：DMG 或 ZIP，将应用移入「应用程序」。使用临时签名，未经 Apple 公证；如被系统拦截，请对可信包在「隐私与安全性」允许打开，并允许局域网访问。不要全局关闭 Gatekeeper。
- `SHA256SUMS` 提供所有安装包的 SHA-256 校验值。

## 升级与已知限制
- 所有设备更新到同一版本。0.2.0 不识别 macOS；0.3.0 包含聊天昵称、内置头像及 macOS 支持。
- 此 Android 发布签名不同于此前本机调试版，无法直接覆盖安装。不要为升级贸然卸载调试版，否则会丢失本地记录；请先妥善备份。之后的正式签名包沿用同一密钥。
- 数据库升级是单向的；升级前备份数据，不要用旧版打开升级后的数据库。
- Windows 未做商业代码签名，macOS 未公证。iOS 暂未支持。
- 仅适用于可信 IPv4 局域网，流量未加密；锁屏、休眠和系统节电可能中断连接。
- 自动构建和测试不代表所有真机组合均已验收，尤其 macOS 的跨设备互传、权限、通知、剪贴板及休眠恢复仍欢迎社区测试。

反馈请附系统版本、对端版本、复现步骤和错误信息，不要公开聊天正文、密钥或私人文件。

源代码：`{args.sha}`；自动构建：{os.environ['GITHUB_SERVER_URL']}/{repository}/actions/runs/{os.environ['GITHUB_RUN_ID']}
"""
    notes_path = directory / "RELEASE-NOTES.md"
    notes_path.write_text(notes, encoding="utf-8")
    existing = subprocess.run(["gh", "release", "view", args.tag, "--repo", repository, "--json", "isDraft"],
                              capture_output=True, text=True, encoding="utf-8")
    if existing.returncode:
        if "release not found" not in existing.stderr.lower():
            raise RuntimeError(existing.stderr)
        command("gh", "release", "create", args.tag, "--repo", repository, "--verify-tag",
                "--draft", "--title", f"NekoSend {version}", "--notes-file", str(notes_path))
    asset_names = names + ["SHA256SUMS"]
    command("gh", "release", "upload", args.tag, "--repo", repository, "--clobber",
            *(str(directory / name) for name in asset_names))
    release = json.loads(command("gh", "release", "view", args.tag, "--repo", repository, "--json", "assets,isDraft"))
    assets = {asset["name"]: asset for asset in release["assets"]}
    for name in asset_names:
        if name not in assets or assets[name]["size"] != (directory / name).stat().st_size:
            raise ValueError(f"Uploaded release asset is missing or truncated: {name}")
        if assets[name].get("digest") != f"sha256:{digest(directory / name)}":
            raise ValueError(f"Uploaded release checksum differs: {name}")
    prerelease = version.startswith("0.") or "-" in version
    command("gh", "release", "edit", args.tag, "--repo", repository, "--draft=false",
            f"--prerelease={str(prerelease).lower()}", "--title", f"NekoSend {version}",
            "--notes-file", str(notes_path))
    print(f"Published {args.tag}: {', '.join(asset_names)}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ["resolve", "prepare", "collect", "publish"]:
        sub = commands.add_parser(name)
        sub.add_argument("--tag", required=True)
        if name != "resolve":
            sub.add_argument("--sha", required=True)
        if name == "collect":
            sub.add_argument("--platform", choices=PACKAGES, required=True)
            sub.add_argument("--certificate-report")
        if name == "publish":
            sub.add_argument("--directory", default="release-output")
    args = parser.parse_args()
    {"resolve": resolve, "prepare": prepare, "collect": collect, "publish": publish}[args.command](args)


if __name__ == "__main__":
    main()
