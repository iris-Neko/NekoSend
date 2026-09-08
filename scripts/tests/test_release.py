import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


spec = importlib.util.spec_from_file_location("release", Path(__file__).parents[1] / "release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def test_version_tag_rejects_shell_and_ref_injection(self):
        for invalid in ["main", "--help", "v0.3.0;echo x", "v0.3.0/../main", "v0.3", ""]:
            with self.assertRaises(ValueError):
                release.version_for(invalid)
        self.assertEqual(release.version_for("v0.3.0"), "0.3.0")
        self.assertEqual(release.version_for("v1.0.0-rc.1"), "1.0.0-rc.1")

    def test_cargo_flutter_and_tag_must_agree(self):
        cargo = '[workspace.package]\nversion = "0.3.0"\n'
        self.assertEqual(release.verify_versions(cargo, "version: 0.3.0+3\n", "0.3.0"), 3)
        for pubspec, version in [("version: 0.2.0+2", "0.3.0"), ("version: 0.3.0+3", "0.4.0"),
                                 ("version: 0.3.0", "0.3.0")]:
            with self.assertRaises(ValueError):
                release.verify_versions(cargo, pubspec, version)

    def make_artifacts(self, directory):
        for platform, names in release.PACKAGES.items():
            files = []
            for name in names:
                content = name.encode()
                (directory / name).write_bytes(content)
                files.append({"name": name, "size": len(content), "sha256": hashlib.sha256(content).hexdigest()})
            (directory / f"manifest-{platform}.json").write_text(json.dumps({
                "tag": "v0.3.0", "sha": "a" * 40, "platform": platform, "files": files,
            }), encoding="utf-8")

    def test_all_platforms_are_required_and_hashes_are_verified(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            self.make_artifacts(directory)
            self.assertEqual(len(release.validate_artifacts(directory, "v0.3.0", "a" * 40)), 6)
            with self.assertRaises(ValueError):
                release.validate_artifacts(directory, "v0.3.0", "b" * 40)
            (directory / "NekoSend-android-arm64.apk").write_bytes(b"tampered")
            with self.assertRaises(ValueError):
                release.validate_artifacts(directory, "v0.3.0", "a" * 40)

    def test_missing_or_unexpected_asset_blocks_publication(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            self.make_artifacts(directory)
            (directory / "private-key.jks").write_bytes(b"must not publish")
            with self.assertRaises(ValueError):
                release.validate_artifacts(directory, "v0.3.0", "a" * 40)
            (directory / "private-key.jks").unlink()
            (directory / "NekoSend-windows-x64.zip").unlink()
            with self.assertRaises(ValueError):
                release.validate_artifacts(directory, "v0.3.0", "a" * 40)

    def test_manifest_cannot_point_outside_artifact_directory(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            self.make_artifacts(directory)
            path = directory / "manifest-linux.json"
            manifest = json.loads(path.read_text())
            manifest["files"][0]["name"] = "../secret"
            path.write_text(json.dumps(manifest))
            with self.assertRaises(ValueError):
                release.validate_artifacts(directory, "v0.3.0", "a" * 40)


if __name__ == "__main__":
    unittest.main()
