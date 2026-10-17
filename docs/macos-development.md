# Stable macOS development app

Use the normal release executable in one development app, rather than generating a new app for every ticket:

- Location: `~/Applications/i3rs Dev.app`
- Bundle identifier: `io.github.friss.i3rs.dev`
- Signing certificate: `i3rs Development` (or a consistently selected existing identity)
- Build: `bash scripts/build-macos-dev.sh`

The bundle contains the actual executable, not a shell launcher or a link to a changing executable outside the bundle. It has its own identity, separate from the distributed `io.github.friss.i3rs` app.

## One-time signing setup

The initial signing check on 2026-10-16 returned no valid code-signing identities. The previous temporary build's designated requirement was a binary hash (`cdhash`), so rebuilding could change its macOS identity.

If you already have an Apple Development or Developer ID Application identity, unlock its keychain and set `I3RS_CODESIGN_IDENTITY` to its name or SHA-1 fingerprint. Keep using the same certificate and signing identifier for subsequent development builds.

For a local development identity, open **Keychain Access > Certificate Assistant > Create a Certificate**. Name it **i3rs Development**, select **Self Signed Root** as Identity Type and **Code Signing** as Certificate Type. Choose an appropriate validity period if overriding defaults, and keep its private key in your login keychain. This is a local signing identity; it does not make a distributable, notarized Developer ID release. If macOS reports the certificate untrusted, inspect that certificate's Trust settings and configure its Code Signing trust as needed; the choice belongs to the user. Do not export the private key or add it to the repository.

See [Apple's certificate creation instructions](https://support.apple.com/guide/keychain-access/create-self-signed-certificates-kyca8916/mac) and [code signing requirements](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements). Consistent designated requirements let macOS recognize an updated app as the same code. A stable path/name alone does not provide that when a signature is tied to a changing binary hash.

Run from the repository:

```bash
bash scripts/build-macos-dev.sh
```

Or select an existing identity:

```bash
I3RS_CODESIGN_IDENTITY='Apple Development: Your Name (TEAMID)' bash scripts/build-macos-dev.sh
```

Keychain may prompt to authorize signing with the private key; respond manually. The script refuses to silently substitute ad-hoc signing if a certificate is unavailable. `--ad-hoc --output /absolute/path/diagnostic.app` is available for packaging diagnostics, but is not the persistent-permission workflow and cannot replace a certificate-signed development app.

## One-time app and macOS grants

These controls are independent:

1. **Computer Use app access:** open `i3rs Dev.app` through Computer Use and select **Always allow** when prompted. Review it in **ChatGPT/Codex Settings > Computer use > Always-allowed apps**. Future agents should select `io.github.friss.i3rs.dev`, rather than a ticket-specific bundle. [OpenAI documentation](https://learn.chatgpt.com/docs/computer-use).
2. **Desktop file access:** this repository and its fixtures currently live under `~/Desktop/Dev/i3rs`. In **System Settings > Privacy & Security > Files & Folders**, enable **Desktop Folder** for **i3rs Dev** when it appears. If a prompt names ChatGPT/Codex or another helper, that process needs its own grant. The bundle includes a Desktop usage description. Selecting a particular file through the Open dialog also grants access to that file, but adjacent sidecars may require additional access. [Apple file-access documentation](https://developer.apple.com/documentation/bundleresources/information-property-list/nsdesktopfolderusagedescription).
3. **Screen visibility and control:** in **System Settings > Privacy & Security**, check **Screen Recording** (sometimes labeled **Screen & System Audio Recording**) and **Accessibility** for **Codex Computer Use**. These permissions belong to the automation helper, not each i3rs build. [OpenAI setup](https://learn.chatgpt.com/docs/computer-use).

The user must approve OS privacy prompts. The build script does not edit TCC, grant Full Disk Access, change Computer Use policies, or change the task's filesystem sandbox. Normal file/shell sandbox rules still apply after macOS permission is granted.

An alternative to Desktop permission is to move development projects and recordings to a location such as `~/Dev/i3rs`, outside macOS's protected Desktop/Documents/Downloads locations, then update the saved project location. Moving this repository is a separate user decision; the build command works from its new location.

## Every subsequent GUI validation

1. Quit the existing **i3rs Dev** instance.
2. Run `bash scripts/build-macos-dev.sh` to rebuild and replace the same bundle, using the same signing certificate.
3. Open the app with Computer Use using the fixed full path, `~/Applications/i3rs Dev.app` expanded to the user's home directory. Bundle-ID lookup can be ambiguous if LaunchServices has indexed a diagnostic copy; remove generated diagnostic bundles after packaging checks.
4. Load the required recording with the app's Open dialog and perform the check.

There is no absolute “forever” guarantee: certificate changes, permission resets, organizational policy or OS changes can require another grant. Keep the identity, location and signing certificate consistent to avoid unnecessary repeat prompts. If prompts recur, compare `codesign --display -r- "$HOME/Applications/i3rs Dev.app"` across builds before resetting permissions.

## Validation of the tooling

The script's shell syntax, missing-certificate failure, and isolated ad-hoc bundle generation/signature verification were checked on macOS.

On 2026-10-16, the user created `i3rs Development` and approved its Code Signing trust. The persistent app was installed at `/Users/friss/Applications/i3rs Dev.app`, signed and verified twice. Both builds produced the same designated requirement:

```text
identifier "io.github.friss.i3rs.dev" and certificate leaf = H"eaf787aad55ebb71b723f35f3608dd169e37cd34"
```

Computer Use opened the signed app before and after rebuilding, and the app successfully loaded `/Users/friss/Desktop/Dev/i3rs/test_data/VIR_LAP.ld` through its Open dialog on both occasions. The generated diagnostic app duplicate was removed after it caused ambiguous bundle-ID lookup. A blanket Desktop-folder grant and the saved Always-allow setting were not independently inspected; this check establishes the normal file-picker workflow. Computer Use access to System Settings was not approved, and no OS privacy settings were changed by automation.
