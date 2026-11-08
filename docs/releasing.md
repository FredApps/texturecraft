# Releasing TextureCraft

The recipe is the [release playbook](release-playbook.md); this page covers what is
specific to TextureCraft. The pipeline is [`.github/workflows/release.yml`](../.github/workflows/release.yml).

## Release manager checklist

The steps below are the release manager's; agents never create the `release` branch, tags or
GitHub Releases. Follow the checklist below and the linked release playbook.

### 1. One-time repository setup (org admin)

Status on 2026-10-23: `storytold/texturecraft` has the `release` environment (deployable from
`release` only) and the `release` ruleset (steps 1 and 2 below). Still to do before the first run:
the secrets (step 3) and the runner group (step 4); the `release` branch is created by the first
release push. An org admin completes the following setup for this repo:

1. Create the **`release` environment**, with deployment branches limited to `release`.
2. Add the **ruleset on `refs/heads/release`** (restrict creations, updates and deletions, block
   force pushes; bypass: the `release-managers` team and the Organization admin role).
3. Set these **environment secrets** on `release` (`gh secret set <NAME> --env release -R
   storytold/texturecraft`). Never as org- or repo-level Actions secrets: only the `release`
   environment keeps them away from other branches.

   | Secret | Read by |
   |---|---|
   | `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD` | macOS: Developer ID certificate into a temporary keychain |
   | `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | macOS: notarization (`notarytool`) and its verification |
   | `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE` | Windows: Azure Trusted Signing (the org's method) |
   | `WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD` | Windows: a `.pfx` instead of Azure; takes precedence when set. Leave unset |

   Those twelve (`APPLE_*` and `KEYCHAIN_PASSWORD`, `AZURE_*`) are all a signed release needs;
   `GITHUB_TOKEN` is provided by Actions.
4. Add the repo to the org's **Default runner group** (Settings → Actions → Runner groups). The
   macOS job (`macos-15-xlarge`) and the FreeBSD job (`github-ubuntu-16core`) run on larger runners,
   which only serve repos in that group; outside it they wait for a runner forever, with no error.

### 2. Set the version

Start from an up-to-date `main` and create a version-bump branch:

```sh
git switch main
git pull --ff-only
git switch -c bump-version-0.2.0
cargo xtask version                 # prints the current version
cargo xtask version set 0.2.0       # or 0.2.0-rc.1 for a pre-release
```

`set` rewrites `[workspace.package] version` in the root `Cargo.toml` (the single source of truth)
and refreshes `Cargo.lock`. Commit the version bump on that branch, push the branch, and open
a pull request into `main`. Wait for the applicable checks to pass and for review, then merge
the pull request. Do not push changes directly to `main`. Wait for CI on the merged `main`
commit to pass before cutting the release below.

### 3. Cut the release

Fast-forward `release` to `main` and push it (only the `release-managers` team and org admins can):

```sh
git fetch origin
git push origin origin/main:refs/heads/release
```

That push runs `release.yml`, and also `freebsd.yml` and `windows-arm64.yml` (build, test, install
checks). A version with a hyphen becomes a pre-release. To test without bumping, dispatch
`release.yml` on `release` with `version` set (e.g. `0.2.0-rc.1`); a dispatch on any other branch
is a dry run (Linux, Flatpak, FreeBSD and web build; the signing jobs are refused by the
environment, so no draft is made).

### 4. Wait for green and verify

```sh
RID=$(gh run list -R storytold/texturecraft -w release.yml -L1 --json databaseId --jq '.[0].databaseId')
gh run watch $RID -R storytold/texturecraft
gh run view $RID -R storytold/texturecraft --log | grep -E "status: Accepted|staple and validate action worked|::warning"
```

- Every job is green, and no `::warning::` names a missing secret (that platform shipped unsigned:
  fix the secret and re-run, don't publish).
- macOS: two `The staple and validate action worked!` lines (app and DMG) and `status: Accepted`.
- The draft `TextureCraft v<version>` exists, targets the commit you pushed, and carries every
  file in the table below plus `SHA256SUMS.txt`.
- Download, check and launch at least one artifact per platform:
  - Windows: `signtool verify /pa /v texturecraft-<v>-windows-x64.msi` succeeds and names the
    org's publisher; install, open a `.tcraft` by double-click, `texturecraft-cli --version` from the
    install dir, uninstall.
  - macOS: `spctl -a -vvv -t install texturecraft-<v>-macos-universal.dmg` says *Notarized
    Developer ID*; the app opens with no Gatekeeper prompt.
  - Linux: `sha256sum -c SHA256SUMS.txt --ignore-missing`, then run the AppImage and install the
    `.deb` or `.rpm`; `texturecraft --version`, `texturecraft-cli --version`.
  - Web: unzip `texturecraft-web-<v>.zip`, serve it (`python3 -m http.server`), open it in a
    WebGPU browser.

### 5. Publish

Review the generated notes, then publish the draft (`gh release edit v<version> -R
storytold/texturecraft --draft=false --latest`, or `--prerelease` for an rc). A draft is updated in
place by a re-run; a published release is never overwritten, so bump the version before building
again.

## What gets built

All file names are `texturecraft-<version>-<platform>-<arch>.<ext>`, written to `dist/release/`.

| Platform | Artifacts | Script |
|---|---|---|
| macOS universal (`macos-15-xlarge`) | `-macos-universal.dmg` (`TextureCraft.app`), `texturecraft-cli-<version>-macos-universal.zip` | `packaging/macos/package.sh`, `verify.sh` |
| Windows x64, x86, arm64 | `-windows-<arch>.msi`, `-windows-<arch>-portable.zip` | `packaging/windows/package.ps1` |
| Linux x86_64, aarch64 (Ubuntu 22.04, glibc 2.35) | `.AppImage` + `.AppImage.zsync`, `.deb`, `.rpm`, `.tar.gz` | `packaging/linux/package.sh` |
| Flatpak x86_64, aarch64 | `-linux-<arch>.flatpak`, repackaged from the Linux tarball | `packaging/linux/flatpak-bundle.sh` |
| FreeBSD x86_64 (14.3 VM on `github-ubuntu-16core`) | `-freebsd-x86_64.tar.gz` | `packaging/freebsd/package.sh` |
| Web | `texturecraft-web-<version>.zip` (trunk, `wasm-release` profile; hosting notes in `packaging/web/README.md`) | `packaging/web/package.sh` |

Every desktop package ships both binaries, `texturecraft` and `texturecraft-cli` (on macOS the CLI
is its own zip next to the DMG). Release builds embed the craft-fonts interface font (Inter) and CJK UI fonts
(`CRAFT_FONTS_REF` in `release.yml`) and ship their `OFL-<family>.txt` licences with the other
notices. The packaging
scripts smoke-test `texturecraft-cli --version`, and the Linux job also runs the AppImage with
`--version`, so both binaries must print their version and exit without opening a window.

## TextureCraft-specific values

| What | Value |
|---|---|
| Bundle / app id | `ai.storyteller.texturecraft` |
| Project format | `.tcraft`, MIME `application/x-texturecraft`, UTI `ai.storyteller.texturecraft.project` (owned) |
| Also opens | `.glb .gltf .fbx .dae .usd .usda .usdc .usdz .abc .obj .ply .stl` (formats `mesh_import::MESH_EXTENSIONS`; "Open with" only, never the default app). Declared in `texturecraft.wxs`, `Info.plist.in`, the `.desktop` file, `mime.xml` and the AppStream metainfo: keep all five in step |
| WiX UpgradeCode | `7E670E60-BB22-4E6B-8749-DD3FD4CA8979`, never change it |
| Flathub manifest | `packaging/linux/flatpak/ai.storyteller.TextureCraft.yml` (release bundle: `…TextureCraft.bundle.yml`) |
| macOS minimum | 11.0 (`LSMinimumSystemVersion`, `MACOSX_DEPLOYMENT_TARGET`) |

## Version and build info in the binaries

`apps/texturecraft/build.rs` (also `include!`d by `apps/texturecraft-cli/build.rs`) sets, for
`env!`:

- `TEXTURECRAFT_BUILD_SHA`: the full commit. CI sets it to `github.sha`; `packaging/env.sh` sets
  it from git; otherwise build.rs asks git itself; empty when unknown.
- `TEXTURECRAFT_BUILD_DATE`: `YYYY-MM-DD` (UTC). CI sets it once per run; otherwise today's date,
  or `SOURCE_DATE_EPOCH`'s for reproducible builds.

On Windows targets it also embeds `assets/app-icon/texturecraft.ico` and VERSIONINFO
(ProductName TextureCraft); `package.ps1` sets `TEXTURECRAFT_REQUIRE_WINRES=1` so a release can't
ship an exe without them. The version itself is `CARGO_PKG_VERSION`.

Other environment variables the scripts read: `TEXTURECRAFT_VERSION` (override the version),
`DIST`, `CARGO_TARGET_DIR`, `TEXTURECRAFT_MAINTAINER` (deb/rpm maintainer),
`TEXTURECRAFT_WASM_MAX_BYTES` (web size gate, default 24 MiB), and at run time
`TEXTURECRAFT_NO_DESKTOP_INTEGRATION=1` (the AppImage's AppRun skips installing its `.desktop`
entry and icons).

## Signing

Secrets live only in the `release` environment and are all optional: a missing one gives an
unsigned artifact and a `::warning::`, never a failed build.

- macOS: `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD` (import into a
  temporary keychain, `packaging/macos/import-cert.sh`), then `APPLE_ID`, `APPLE_PASSWORD`,
  `APPLE_TEAM_ID` for `notarytool` and stapling. Without them the app is ad-hoc signed.
- Windows: `packaging/windows/sign.ps1` uses a PFX (`WINDOWS_CERTIFICATE`,
  `WINDOWS_CERTIFICATE_PASSWORD`) if set, otherwise Azure Trusted Signing (all six `AZURE_*`).
  It signs both exes before WiX and the MSI after.

## Icons

`assets/app-icon/texturecraft.svg` is the engraved zebra (the owner's original drawing, vectorised
to three colours); `texturecraft-small.svg` is a lighter
trace for the hicolor scalable icon. `packaging/icons.sh` renders everything else with resvg and
`cargo xtask ico|icns`: the hicolor PNGs (16-512 px) and scalable SVG, `texturecraft-1024.png`,
`texturecraft.ico` and `texturecraft.icns`. The outputs are committed; rerun it after changing the
artwork and keep `ATTRIBUTION.md` in step. See `assets/app-icon/README.md`.

## Local checks

- `cargo xtask ci`: fmt, clippy, tests, `cargo xtask layers`, `cargo xtask wasm`.
- `bash packaging/linux/apprun-test.sh` and `bash packaging/freebsd/package.sh --dry-run` run
  anywhere with bash.
- `packaging/macos/package.sh --arch aarch64` on a Mac builds an ad-hoc signed DMG.
- `pwsh packaging/windows/package.ps1 -Arch x64` on Windows (needs WiX 5.0.2) builds an unsigned
  MSI and portable zip. Use PowerShell 7 as CI does: Windows PowerShell 5.1 with redirected output
  turns cargo's stderr into an error, so there build with cargo first and pass `-SkipBuild`.
- `.github/workflows/packaging-lint.yml` runs actionlint, shellcheck, the PowerShell parser and
  the desktop/AppStream/Flatpak validators on every change to `packaging/` or the workflows.
