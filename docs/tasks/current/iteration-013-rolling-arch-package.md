# Iteration 013 — Restore the rolling Arch package build

**Status:** Planned

## Sources

- [Issue #17 — v0.2.0 PKGBUILD fails on rolling Arch with Rust 1.100.0](https://github.com/mateuszkowalczyk/ancs-bridge/issues/17)
- [Packaging requirements](../../prd/03-packaging-security.md#aur-package)
- [Packaging validation](../../prd/04-validation-release.md#packaging-tests)

## Tasks

- [x] Update `bluer` to `0.17.4` in `Cargo.toml` and `Cargo.lock` without refreshing unrelated dependencies; verify the locked release build and tests with current Arch Rust.
- [x] Isolate the `doctor` integration test from the build host's system D-Bus so `makepkg` does not depend on an active BlueZ daemon.
- [x] Prepare package `0.2.0-2` without changing the CLI version: pin the fixed source commit and archive SHA-256 in `PKGBUILD`, and regenerate `.SRCINFO`.
- [x] In a clean x86_64 Arch QEMU/KVM guest, run a full system update and record the Rust/Cargo package versions. Verify the source checksum and `.SRCINFO`, build the final recipe with `makepkg --cleanbuild --syncdeps` as an unprivileged user, run `namcap` on the recipe and package, and install the package. Confirm the installed `ancs-bridge version` returns `{"apiVersion":1,"version":"0.2.0"}`.
- [ ] Install that exact package on the physical Arch machine. With a real iPhone, verify notification delivery before and after reconnect, including the `notify_io()` path used when BlueZ reports `ServicesResolved=false`. If that path regresses, diagnose before publishing; consider backporting only the upstream compiler fix.
- [x] Document the clean rolling Arch build/install check as a required pre-publication step for future Arch package releases.
- [ ] After the hardware smoke passes, publish the corrected recipe under immutable tag `arch-0.2.0-2` and update the GitHub `v0.2.0` release notes to direct users to it. Verify the public install link; do not move the original tag.

## Out of scope

- Scheduled CI and a Rust beta matrix.

## Validation notes

- Pre- and post-review QEMU builds passed with Arch `rust 1:1.100.0-1` (providing Rust and Cargo 1.100.0): 116 tests passed, two hardware tests ignored, and the installed CLI reported version `0.2.0` and API `1`.
- `namcap` reported no errors. Its loader and implicit `glibc`/`libgcc` warnings match the prior packaging review; BlueZ and WirePlumber are intentional runtime dependencies, not linked libraries.
- The exported post-review package has SHA-256 `71ea0d5024b5ef1be0b5f6257e3fdb28d4774d49dd815826f41d7b8d6519f282`. Use these exact bytes for the physical smoke test.
