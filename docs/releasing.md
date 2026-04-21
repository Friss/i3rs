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
- cross-platform workspace builds on Ubuntu, macOS, and Windows

## Main Release Flow

Every push to `main` runs the release workflow. It:

1. Re-runs formatting, linting, tests, and the `i3rs-core` crates.io dry run.
2. Checks whether the version tag already exists.
3. Publishes `i3rs-core` and `i3rs-cli` to crates.io when `CARGO_REGISTRY_TOKEN` is configured.
4. Packages desktop artifacts with `cargo-packager`.
5. Creates a GitHub release for the current workspace version.

If the tag for the current version already exists, the workflow stops after verification and skips crates.io publishing, packaging, and GitHub release creation.

## Required Secrets

Set this repository secret before enabling crates.io publishing:

- `CARGO_REGISTRY_TOKEN`: crates.io API token with publish access

If the secret is missing, the workflow still builds and publishes GitHub release artifacts, but skips crates.io publishing.

## Versioning

The release workflow reads the version from [`Cargo.toml`](/Users/friss/Desktop/i3rs/Cargo.toml) under `workspace.package.version` and uses:

- crates.io version: `0.1.0`
- GitHub release tag: `v0.1.0`

To cut a new release:

1. Bump `workspace.package.version` in [`Cargo.toml`](/Users/friss/Desktop/i3rs/Cargo.toml).
2. Merge the change to `main`.
3. Let the `Release Main` workflow publish crates and attach packaged binaries.

## crates.io Scope

The crates.io release scaffold currently targets:

- [`i3rs-core`](/Users/friss/Desktop/i3rs/crates/i3rs-core/Cargo.toml)
- [`i3rs-cli`](/Users/friss/Desktop/i3rs/crates/i3rs-cli/Cargo.toml)

The desktop GUI crate [`i3rs-app`](/Users/friss/Desktop/i3rs/crates/i3rs-app/Cargo.toml) is marked `publish = false` and is distributed through packaged desktop artifacts instead of crates.io.
