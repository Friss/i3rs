"""Combine the landing page and a fresh Trunk bundle for /demo/."""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import shutil
import subprocess


def assemble(web_dist: Path, output: Path, commit: str) -> None:
    website = Path(__file__).resolve().parent
    html = (web_dist / "index.html").read_text()
    wasm_paths = re.findall(r"['\"](/demo/[^'\"]+\.wasm)['\"]", html)
    js_paths = re.findall(r"['\"](/demo/[^'\"]+\.js)['\"]", html)
    if not wasm_paths or not js_paths:
        raise ValueError("Build the app with trunk build --release --public-url /demo/ --locked")
    for url in set(wasm_paths + js_paths):
        if not (web_dist / Path(url).name).is_file():
            raise ValueError(f"Missing web bundle asset: {url}")
    for sample in ["VIR_LAP.ld", "VIR_LAP.ldx", "i3rs.ico"]:
        if not (web_dist / sample).is_file():
            raise ValueError(f"Missing bundled sample asset: {sample}")
    if "i3rsLoadBundledSample" not in html or "i3rsConfigureVirSample" not in html:
        raise ValueError("The web bundle is missing the track annotation sample setup")

    # Validate before replacing an existing build, and never copy stale hashed assets.
    if output.exists():
        shutil.rmtree(output)
    shutil.copytree(website / "public", output)
    shutil.copytree(web_dist, output / "demo")
    build = {
        "commit": commit,
        "branch": os.environ.get("WORKERS_CI_BRANCH", os.environ.get("GITHUB_HEAD_REF")
                                 or os.environ.get("GITHUB_REF_NAME", "local")),
        "preserved_fixes": ["track-annotations"],
        "built_at": datetime.now(timezone.utc).isoformat(),
    }
    (output / "demo/build.json").write_text(json.dumps(build, indent=2) + "\n")


def main() -> None:
    website = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--web-dist", type=Path, default=website.parent / "crates/i3rs-app/dist")
    parser.add_argument("--output", type=Path, default=website / "dist")
    parser.add_argument("--commit")
    args = parser.parse_args()
    commit = args.commit or subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=website, text=True
    ).strip()
    assemble(args.web_dist.resolve(), args.output.resolve(), commit)
    print(f"Assembled i3rs.boxbox.dev for {commit}")


if __name__ == "__main__":
    main()
