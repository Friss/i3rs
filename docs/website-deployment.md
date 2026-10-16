# Website deployment

[i3rs.boxbox.dev](https://i3rs.boxbox.dev) is the `i3rs-boxbox-dev` Cloudflare Worker.
Its landing page, images, Worker, and build settings live in `website/` in this
repository. The [Deploy Website workflow](../.github/workflows/deploy-website.yml)
runs on every push to **main** and calls the `github-main` Cloudflare deploy hook.
Workers Builds then runs:

```bash
# Build command, from the repository root:
bash website/build.sh

# Deploy command, only after the build succeeds:
npm --prefix website run deploy
```

The production hook can build only the `main` branch; preview deployments are
disabled. Its URL is stored as the encrypted GitHub repository secret
`CLOUDFLARE_DEPLOY_HOOK`. Workers Builds supplies its existing deployment token.
Never commit the hook URL or a Cloudflare API token, or include them in the browser
bundle. Deployments do not depend on a desktop release tag or version bump.

`website/build.sh` installs the tested Rust 1.97.1 toolchain and a checksum-verified
Trunk 0.21.14 binary when needed, installs the locked Wrangler dependency, builds
the app with `/demo/` as its public URL, assembles the landing page and fresh web
bundle, and validates the Worker with a dry run before deployment. The existing
Worker compatibility date and cross-origin isolation headers are preserved.

## Track annotations

The annotation fixes are regular Rust and web source, rather than a patch applied
during deployment. The bundled sample waits for corrected GPS data, selects the
complete VIR lap, loads all 33 distance sections, and enables section names,
corner minima, straight maxima, and gear changes. Dense labels use additional
columns instead of overlapping. Both the embedded sample and its full-screen
sample button use the same loading path.

The Rust tests `bundled_vir_sample_waits_for_corrections_then_prepares_complete_track_annotations`
and `dense_annotations_keep_every_label_readable_in_short_embedded_panels` protect
these behaviors in the existing PR verification workflow. Production builds also
run the website assembly tests and reject bundles missing the sample annotation
setup before deployment.

## Verify or rebuild

`https://i3rs.boxbox.dev/demo/build.json` identifies the deployed Git commit and
build time. After a push to `main`, the **Deploy Website** GitHub Actions run
confirms that Cloudflare accepted the build request. Check the Worker's **Builds**
tab in Cloudflare for completion, then compare that file with the commit you pushed. A failed build leaves the
previous deployment serving traffic. Use **Retry build** in Cloudflare to rebuild
the same commit; the build log includes the pinned tool versions and deployment
result. You can also dispatch **Deploy Website** manually on `main` to rebuild
without a new commit.

For a local build, run the same build command above, then `npm --prefix website
run dev`. The generated website is `website/dist/`; generated files
and dependencies are ignored. `website/public/` is the landing page source, while
`crates/i3rs-app/` owns the browser application.
