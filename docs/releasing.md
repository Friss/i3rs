# Releasing

i3rs now has two release automation paths:

- PR verification via [`.github/workflows/pr-verify.yml`](../.github/workflows/pr-verify.yml)
- Main-branch release publishing via [`.github/workflows/release-main.yml`](../.github/workflows/release-main.yml)

## PR Verification

Every pull request runs:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo publish -p i3rs-core --dry-run`
- `trunk build --release` for `crates/i3rs-app`
- cross-platform workspace builds on Ubuntu, macOS, and Windows

## Main Release Flow

Every push to `main` runs the release workflow. It:

1. Re-runs formatting, linting, tests, and the `i3rs-core` crates.io dry run.
2. Checks whether the crate release tag and app package release tag already exist.
3. Publishes `i3rs-core` and `i3rs-cli` to crates.io when `CARGO_REGISTRY_TOKEN` is configured and the crate tag is new.
4. Packages desktop artifacts with `cargo-packager` when the app package tag is new.
5. Builds the web bundle with Trunk as another packaging matrix variant and archives it as `i3rs-web.tar.gz`.
6. Creates a GitHub release for the current app package version.

If the crate tag already exists, the workflow skips crates.io publishing for that version.
If the app package tag already exists, the workflow skips packaging and GitHub release creation for that version.

## Required Secrets

Set this repository secret before enabling crates.io publishing:

- `CARGO_REGISTRY_TOKEN`: crates.io API token with publish access

If the secret is missing, the workflow still builds and publishes GitHub release artifacts, but skips crates.io publishing.

## Versioning

The release workflow now tracks crate and app package releases separately:

- Crates.io version source: [`Cargo.toml`](../Cargo.toml) `workspace.package.version`
- Crates.io tag: `v0.1.0`
- App package version source: [`crates/i3rs-app/Packager.toml`](../crates/i3rs-app/Packager.toml) `version`
- GitHub packaged release tag: `app-v0.1.0`

To cut a new crates.io release:

1. Bump `workspace.package.version` in [`Cargo.toml`](../Cargo.toml).
2. Merge the change to `main`.
3. Let the `Release Main` workflow publish crates and create the crate tag.

To cut a new packaged app release:

1. Bump `version` in [`crates/i3rs-app/Packager.toml`](../crates/i3rs-app/Packager.toml).
2. Merge the change to `main`.
3. Let the `Release Main` workflow package binaries, build the web bundle, and create a GitHub release.

## crates.io Scope

The crates.io release scaffold currently targets:

- [`i3rs-core`](../crates/i3rs-core/Cargo.toml)
- [`i3rs-cli`](../crates/i3rs-cli/Cargo.toml)

The desktop GUI crate [`i3rs-app`](../crates/i3rs-app/Cargo.toml) is marked `publish = false` and is distributed through packaged desktop artifacts instead of crates.io.

## Web Bundle

The release workflow now also publishes a web bundle artifact:

- GitHub release asset: `i3rs-web.tar.gz`
- Build source: [`crates/i3rs-app`](../crates/i3rs-app)
- Build command: `trunk build --release`

This archive contains the static web output from `crates/i3rs-app/dist/` and can be unpacked into another web project or static hosting setup.
