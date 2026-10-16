#!/bin/bash
# Build the normal app into one consistently identified development bundle.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/build-macos-dev.sh [--output /absolute/path/i3rs\ Dev.app] [--ad-hoc]

Default destination: ~/Applications/i3rs Dev.app
Default signing identity: i3rs Development
Override the certificate with I3RS_CODESIGN_IDENTITY (name or SHA-1 fingerprint).

Quit i3rs Dev before rebuilding, then relaunch it after the build.
--ad-hoc is for packaging diagnostics only; it does not preserve macOS grants
across changed builds. See docs/macos-development.md for the one-time setup.
EOF
}

if [[ $(uname -s) != Darwin ]]; then
    echo 'This development bundle requires macOS.' >&2
    exit 1
fi

project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
destination="$HOME/Applications/i3rs Dev.app"
signing_identity=${I3RS_CODESIGN_IDENTITY:-i3rs Development}
bundle_id=io.github.friss.i3rs.dev
ad_hoc=false
while [[ $# -gt 0 ]]; do
    case "$1" in
        --output)
            if [[ $# -lt 2 ]]; then usage >&2; exit 2; fi
            destination=$2
            shift 2
            ;;
        --ad-hoc) ad_hoc=true; shift ;;
        --help|-h) usage; exit 0 ;;
        *) usage >&2; exit 2 ;;
    esac
done
case "$destination" in
    /*.app) ;;
    *) echo '--output must be an absolute .app path.' >&2; exit 2 ;;
esac
if [[ $ad_hoc == true ]]; then
    signing_identity=-
    echo 'Diagnostic ad-hoc signing: changed builds may require new macOS permissions.' >&2
elif [[ $signing_identity == - ]]; then
    echo 'Use --ad-hoc explicitly for diagnostic builds, not the persistent development app.' >&2
    exit 2
else
    identities=$(/usr/bin/security find-identity -v -p codesigning)
    case "$identities" in
        *\""$signing_identity"\"*|*" $signing_identity "*) ;;
        *)
            echo "Code-signing identity '$signing_identity' is unavailable." >&2
            echo 'Create/unlock it in Keychain Access, or set I3RS_CODESIGN_IDENTITY.' >&2
            echo "Setup: $project_root/docs/macos-development.md" >&2
            exit 2
            ;;
    esac
fi

if [[ -e "$destination" || -L "$destination" ]]; then
    existing_id=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$destination/Contents/Info.plist" 2>/dev/null || true)
    if [[ $existing_id != "$bundle_id" || -L "$destination" ]]; then
        echo "Refusing to replace a different app or symlink: $destination" >&2
        exit 2
    fi
    if [[ $ad_hoc == true ]]; then
        existing_signature=$(/usr/bin/codesign --display --verbose=2 "$destination" 2>&1 || true)
        if [[ $existing_signature != *Signature=adhoc* ]]; then
            echo 'Refusing to replace a certificate-signed app with an ad-hoc build.' >&2
            exit 2
        fi
    fi
fi

cd "$project_root"
cargo build --release -p i3rs-app
build_directory=${CARGO_TARGET_DIR:-$project_root/target}
if [[ $build_directory != /* ]]; then build_directory="$project_root/$build_directory"; fi
parent=$(dirname "$destination")
mkdir -p "$parent"
staging_dir=$(mktemp -d "$parent/.i3rs-dev-build.XXXXXX")
trap 'rm -rf "${staging_dir:?}"' EXIT
staged_app="$staging_dir/i3rs Dev.app"
mkdir -p "$staged_app/Contents/MacOS" "$staged_app/Contents/Resources"
cp "$build_directory/release/i3rs-app" "$staged_app/Contents/MacOS/i3rs-app"
cp crates/i3rs-app/packaging/icons/i3rs.icns "$staged_app/Contents/Resources/i3rs.icns"
cat > "$staged_app/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
    <key>CFBundleIdentifier</key><string>io.github.friss.i3rs.dev</string>
    <key>CFBundleName</key><string>i3rs Dev</string>
    <key>CFBundleDisplayName</key><string>i3rs Dev</string>
    <key>CFBundleExecutable</key><string>i3rs-app</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleVersion</key><string>1</string>
    <key>CFBundleShortVersionString</key><string>0.0.0</string>
    <key>CFBundleIconFile</key><string>i3rs.icns</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSDesktopFolderUsageDescription</key>
    <string>i3rs opens telemetry recordings, sidecars and workspaces in your development project on the Desktop.</string>
</dict></plist>
EOF
/usr/bin/plutil -lint "$staged_app/Contents/Info.plist"
/usr/bin/codesign --force --sign "$signing_identity" --identifier "$bundle_id" --timestamp=none "$staged_app"
/usr/bin/codesign --verify --strict --verbose=2 "$staged_app"

if [[ -e "$destination" && $ad_hoc == false ]]; then
    existing_signature=$(/usr/bin/codesign --display --verbose=2 "$destination" 2>&1 || true)
    if [[ $existing_signature != *Signature=adhoc* ]]; then
        previous_requirement=$(/usr/bin/codesign --display -r- "$destination" 2>&1 | sed -n 's/^.*designated => //p')
        next_requirement=$(/usr/bin/codesign --display -r- "$staged_app" 2>&1 | sed -n 's/^.*designated => //p')
        if [[ -z $previous_requirement || $previous_requirement != "$next_requirement" ]]; then
            echo 'The signing requirement changed; the existing development app was not replaced.' >&2
            echo 'Keep using its original certificate to retain its macOS identity.' >&2
            exit 2
        fi
    fi
fi

if [[ -e "$destination" ]]; then mv "$destination" "$staging_dir/previous.app"; fi
if ! mv "$staged_app" "$destination"; then
    if [[ -d "$staging_dir/previous.app" ]]; then mv "$staging_dir/previous.app" "$destination"; fi
    exit 1
fi
printf 'Built: %s\nComputer Use app ID: %s\n' "$destination" "$bundle_id"
/usr/bin/codesign --display -r- "$destination" 2>&1
echo 'Relaunch i3rs Dev to use this build. OS/app approvals are not changed by this script.'
