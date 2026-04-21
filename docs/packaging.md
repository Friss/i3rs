# Packaging

i3rs uses [`cargo-packager`](https://github.com/crabnebula-dev/cargo-packager) for desktop packaging.
The repository config lives in [crates/i3rs-app/Packager.toml](../crates/i3rs-app/Packager.toml).

## Supported Targets

- macOS: `.app` bundle and `.dmg`
- Linux: `.AppImage`
- Windows: `.msi` via WiX

These match the Milestone 8 packaging targets in [`docs/PLAN.md`](../docs/PLAN.md).

## Prerequisites

1. Install the packager CLI:

```bash
cargo install cargo-packager --locked
```

2. Build on the target OS for the package you want:

- macOS is required for `.app` / `.dmg`
- Linux is required for `.AppImage`
- Windows is required for `.msi`

3. For Windows MSI builds, install the WiX Toolset and make sure it is available on `PATH`.

## Commands

Run packaging commands from [`crates/i3rs-app`](../crates/i3rs-app):

### macOS

```bash
cd crates/i3rs-app
cargo packager --release --formats app
cargo packager --release --formats dmg
```

### Linux

```bash
cd crates/i3rs-app
cargo packager --release --formats appimage
```

### Windows

```powershell
Set-Location crates/i3rs-app
cargo packager --release --formats wix
```

## Output

Generated artifacts are written to:

```text
dist/packager/
```

## Packaging Assets

- App icons: [`crates/i3rs-app/packaging/icons`](../crates/i3rs-app/packaging/icons)
- Packager config: [`crates/i3rs-app/Packager.toml`](../crates/i3rs-app/Packager.toml)
- License file: [`LICENSE`](../LICENSE)

## Notes

- The packager config currently targets the GUI application only.
- Packaging is intentionally documented as platform-native rather than cross-compiling from a single host.
- If we later automate releases in GitHub Actions, this document should stay the source of truth for expected package formats and prerequisites.
