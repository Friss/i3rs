import json
from pathlib import Path
import tempfile
import unittest

from assemble import assemble


class AssembleTests(unittest.TestCase):
    def bundle(self, root):
        bundle = root / "bundle"
        bundle.mkdir()
        (bundle / "index.html").write_text(
            '<script>import "/demo/app.js"; const wasm="/demo/app.wasm"; '
            "window.i3rsLoadBundledSample; window.i3rsConfigureVirSample;</script>"
        )
        for name in ["app.js", "app.wasm", "VIR_LAP.ld", "VIR_LAP.ldx", "i3rs.ico"]:
            (bundle / name).write_bytes(b"fixture")
        return bundle

    def test_combines_landing_and_sample_and_removes_old_bundle(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle = self.bundle(root)
            output = root / "site"
            (output / "demo").mkdir(parents=True)
            (output / "demo/stale.wasm").touch()
            assemble(bundle, output, "abc123")
            self.assertTrue((output / "index.html").is_file())
            self.assertTrue((output / "main.js").is_file())
            self.assertTrue((output / "demo/app.wasm").is_file())
            self.assertFalse((output / "demo/stale.wasm").exists())
            build = json.loads((output / "demo/build.json").read_text())
            self.assertEqual(build["commit"], "abc123")
            self.assertEqual(build["preserved_fixes"], ["track-annotations"])

    def test_rejects_wrong_public_url_without_replacing_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle = self.bundle(root)
            html = bundle / "index.html"
            html.write_text(html.read_text().replace("/demo/", "/"))
            output = root / "site"
            output.mkdir()
            (output / "previous-build").touch()
            with self.assertRaisesRegex(ValueError, "public-url"):
                assemble(bundle, output, "abc123")
            self.assertTrue((output / "previous-build").exists())

    def test_rejects_a_bundle_without_sample_annotation_setup(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle = self.bundle(root)
            html = bundle / "index.html"
            html.write_text(html.read_text().replace("i3rsConfigureVirSample", "missing"))
            with self.assertRaisesRegex(ValueError, "annotation"):
                assemble(bundle, root / "site", "abc123")


if __name__ == "__main__":
    unittest.main()
