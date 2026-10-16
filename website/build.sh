#!/usr/bin/env bash
set -euo pipefail

site_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$site_root"
export PATH="$HOME/.cargo/bin:$PATH"
# Some CI hosts use NO_COLOR=1, but Trunk's CLI expects a boolean.
export NO_COLOR=true

# Workers Builds does not preinstall Rust. Pin the tested compiler and use
# Trunk's prebuilt release rather than compiling the build tool on every push.
site_rust_version=1.97.1
site_trunk_version=0.21.14
if ! command -v rustup >/dev/null 2>&1; then
  site_rust_installer="$(mktemp)"
  curl --fail --silent --show-error --retry 3 https://sh.rustup.rs -o "$site_rust_installer"
  sh "$site_rust_installer" -y --profile minimal --default-toolchain "$site_rust_version"
  rm -f "$site_rust_installer"
fi
if command -v rustc >/dev/null 2>&1 \
    && [[ "$(rustc --version)" == "rustc $site_rust_version "* ]] \
    && rustup target list --installed | grep -qx wasm32-unknown-unknown; then
  site_rust_toolchain="$(rustup show active-toolchain | cut -d ' ' -f 1)"
else
  rustup toolchain install "$site_rust_version" --profile minimal --target wasm32-unknown-unknown
  site_rust_toolchain="$site_rust_version"
fi

if ! command -v trunk >/dev/null 2>&1 || [[ "$(trunk --version)" != "trunk $site_trunk_version" ]]; then
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) site_trunk_target=x86_64-unknown-linux-gnu ;;
    Linux-aarch64) site_trunk_target=aarch64-unknown-linux-gnu ;;
    Darwin-arm64) site_trunk_target=aarch64-apple-darwin ;;
    Darwin-x86_64) site_trunk_target=x86_64-apple-darwin ;;
    *) echo "Install Trunk $site_trunk_version for this platform before building." >&2; exit 1 ;;
  esac
  site_tools="$site_root/target/website-tools"
  mkdir -p "$site_tools"
  site_trunk_archive="trunk-$site_trunk_target.tar.gz"
  site_trunk_release="https://github.com/trunk-rs/trunk/releases/download/v$site_trunk_version"
  curl --fail --silent --show-error --location --retry 3 "$site_trunk_release/$site_trunk_archive" -o "$site_tools/$site_trunk_archive"
  curl --fail --silent --show-error --location --retry 3 "$site_trunk_release/$site_trunk_archive.sha256" -o "$site_tools/$site_trunk_archive.sha256"
  python3 - "$site_tools/$site_trunk_archive" <<'PY'
import hashlib, pathlib, sys
archive = pathlib.Path(sys.argv[1])
expected = pathlib.Path(str(archive) + ".sha256").read_text().split()[0]
if hashlib.sha256(archive.read_bytes()).hexdigest() != expected:
    raise SystemExit("Trunk release checksum does not match")
PY
  tar -xzf "$site_tools/$site_trunk_archive" -C "$site_tools" trunk
  export PATH="$site_tools:$PATH"
fi

npm --prefix website ci
python3 -m unittest discover -s website -p 'test_*.py'
(cd crates/i3rs-app && RUSTUP_TOOLCHAIN="$site_rust_toolchain" trunk build --release --public-url /demo/ --locked)
python3 website/assemble.py
npm --prefix website run check
