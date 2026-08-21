# Iteration 008 — v0.1.0 release and AUR publication

**Status:** Planned

## Sources

- `docs/prd/03-packaging-security.md`
- `docs/prd/04-validation-release.md`
- `docs/specs/systemd-user-service.md`
- `docs/tasks/archived/iteration-007-field-validation-and-code-audit.md`

## Dependencies and guardrails

- The upstream GitHub repository is currently private and has no release tags.
  AUR publication requires a publicly downloadable immutable source archive.
- The `ancs-bridge` AUR package name was available when this iteration was
  planned; recheck it immediately before publication.
- `makepkg` and Arch `devtools` are installed locally, but `namcap` is not.
  Installing missing host tooling and any privileged package operation require
  an explicit operator step.
- Request separate just-in-time user approval before making the GitHub
  repository public, publishing the `v0.1.0` tag/release, or pushing the initial
  AUR repository. Never rewrite a published release tag or AUR history.
- Release and package installation must not enable/start the service, pair a
  device, alter user configuration, or restart WirePlumber. Existing configured
  user state is preserved across package install, upgrade, and removal. Setup
  always applies the phone-audio policy; the package does not run setup.

## Tasks

### Concise end-user README

- [x] Rewrite `README.md` to open with a brief description of who the bridge is for and why it is useful, including local read-only forwarding through Apple's native ANCS protocol, no cloud/telemetry, and the security/privacy advantages.
- [x] Add a short AUR-first quick start for an end user who only wants to install, confirm the iPhone during setup, enable the user service, and check that it is ready; keep teardown and the optional phone-audio behavior discoverable without exposing internal implementation detail.
- [x] Add one short integration section covering the stable JSON/JSONL commands and systemd user service, then retain only essential platform support, troubleshooting, development, license, and experimental-adapter caveats; remove duplicated architecture, protocol, manual-install, and exhaustive acceptance detail from the landing page.
- [x] Make phone-audio suppression always-on for this unreleased v0.1.0: remove the setup flag and desktop config switch, require WirePlumber for configured operation, and keep package installation itself non-mutating.
- [x] Make JSON the unconditional machine output for setup, diagnostics, status, and version without a redundant format flag, while retaining setup's line-delimited streaming protocol.
- [x] Verify every retained README command and claim against the packaged binary, public installation layout, machine API fixtures, and current specifications, with concise wording and working links.

### Immutable v0.1.0 source release

- [x] Prepare a clean, committed `0.1.0` release candidate with consistent Cargo metadata, lockfile, MIT license, service documentation URL, concise README, release notes, and the complete automated validation gate passing.
- [x] After explicit approval, make the upstream GitHub repository public and verify anonymous access to the source, license, default branch, and security/privacy documentation without exposing local configuration, notification content, or other private artifacts.
- [x] After explicit approval, create and publish the immutable annotated `v0.1.0` tag and GitHub release at the verified release-candidate commit, then record the public archive URL and SHA-256 digest without moving or replacing the tag.

### Source-built Arch package

- [x] Add an AUR-ready `PKGBUILD` for `ancs-bridge` 0.1.0 and generate matching `.SRCINFO`, using only the immutable public release archive and pinned checksum, `cargo` as a make dependency, verified runtime dependencies, and WirePlumber as an accurately described runtime dependency.
- [x] Follow current Arch Rust packaging guidance: prefetch the locked target dependency graph in `prepare()`, build and test frozen/offline in later phases, and install only `/usr/bin/ancs-bridge`, the MIT license, and `ancs-bridge.service` under the systemd user-unit directory.
- [x] Add deterministic packaging checks that reject stale `.SRCINFO`, checksum/source drift, unexpected installed paths, install hooks, automatic service activation, user-state mutation, bundled prebuilt binaries, or a build that does not use the committed lockfile.
- [ ] Run `makepkg --cleanbuild`, `namcap` on both recipe and package, and a clean Arch chroot build; resolve every actionable error and document any narrowly justified false-positive warning.

### Package lifecycle and publication

- [ ] In disposable clean Arch environments, verify clean install, a temporary prior-package-to-0.1.0 upgrade, and removal: package-owned files change exactly as expected while configuration, service enablement intent, Bluetooth bonds, and user-level audio policy remain untouched.
- [ ] Install the final package on the validated host without changing existing enablement, confirm the service returns to `ready`, forward one notification exactly once, and repeat the metadata-only privacy scan against package/build logs, status, journal, configuration, and installed artifacts.
- [ ] Recheck AUR name availability and SSH access, create a clean initial AUR commit containing only required package-repository files, and verify its `.SRCINFO` exactly matches the validated `PKGBUILD`.
- [ ] After explicit approval, push the initial `ancs-bridge` package to the AUR, verify the public package page, metadata, sources, checksum, and clean source build, and leave both upstream and AUR repositories in a reproducible, documented state.

### Final release verification

- [ ] Confirm the public GitHub release, release archive digest, upstream packaging copies, AUR files/page, built package contents, installed version output, enabled/running service, ANCS `ready` state, notification delivery, audio policy, and privacy evidence all describe the same immutable `0.1.0` release; run formatting, warnings-denied Clippy, all non-hardware tests, dependency audit, release build, unit verification, and `git diff --check` once more.

## Implementation notes

- The README now leads with local security and privacy benefits, uses `yay` for
  a short step-by-step AUR and iPhone quick start, explains why Arch package
  installation does not run pairing/setup automatically, and keeps the
  JSON/JSONL integration contract concise.
- With explicit user approval, phone-audio suppression is now mandatory after
  setup. The unreleased config schema no longer stores a desktop preference;
  setup always applies both WirePlumber rules, teardown always removes them,
  configured diagnostics require WirePlumber, and `--disable-phone-audio` is
  removed from the CLI and documentation. The user's local config was migrated
  by removing its obsolete `[desktop]` section.
- With explicit user approval, the redundant `--json` flag was removed before
  release. Machine commands emit JSON unconditionally, while setup still
  streams one JSON object per line and accepts line-delimited JSON commands.
- The committed `0.1.0` candidate passed the release build, formatting,
  warnings-denied Clippy, all non-hardware tests, dependency audit with the
  documented target-inactive advisory ignores, staged-install artifact checks,
  and systemd unit validation. Public archive and AUR tasks remain gated on
  the future immutable GitHub release.
- The repository was made public by the user and verified anonymously. The
  public default branch exposes the intended source, README, license, and
  security/runtime documentation without local configuration or notification
  content.
- The pinned public archive checksum was verified as
  `936d88b31a4675d11d349fd6b6a498f459a2ccb82a7b21927a47c111b8c8515a`.
  `PKGBUILD` and `.SRCINFO` now use that archive, prefetch the locked Cargo
  graph, build/test frozen and offline, install only the three intended paths,
  and disable debug subpackage generation. The recipe was built successfully
  in an isolated AUR-style checkout with dependency checks bypassed because
  this host lacks Arch's packaged `cargo`; the recipe still declares `cargo`
  as a make dependency.
- The immutable GitHub release is published at
  `https://github.com/mateuszkowalczyk/ancs-bridge/releases/tag/v0.1.0`; its
  archive URL is
  `https://github.com/mateuszkowalczyk/ancs-bridge/archive/refs/tags/v0.1.0.tar.gz`.
- `namcap` is not installed locally, and a privileged clean Arch chroot build
  remains an operator-gated check.
- The AUR RPC currently reports no existing `ancs-bridge` package. The AUR SSH
  endpoint could not yet be verified because this host has no accepted
  `aur.archlinux.org` host key; configure and verify SSH before the initial
  AUR repository step.
