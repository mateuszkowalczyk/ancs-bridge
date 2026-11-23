# Iteration 013 — Restore the rolling Arch package build

**Status:** Planned

## Sources

- [Issue #17 — v0.2.0 PKGBUILD fails on rolling Arch with Rust 1.100.0](https://github.com/mateuszkowalczyk/ancs-bridge/issues/17)
- [Packaging requirements](../../prd/03-packaging-security.md#aur-package)
- [Packaging validation](../../prd/04-validation-release.md#packaging-tests)

## Tasks

- [ ] Update `bluer` to `0.17.4` in `Cargo.toml` and `Cargo.lock` without refreshing unrelated dependencies; verify the locked release build and tests with current Arch Rust.
- [ ] Prepare package `0.2.0-2` without changing the CLI version: pin the fixed source commit and archive SHA-256 in `PKGBUILD`, and regenerate `.SRCINFO`.
- [ ] In a clean x86_64 Arch QEMU/KVM guest, run a full system update and record the Rust/Cargo package versions. Verify the source checksum and `.SRCINFO`, build the final recipe with `makepkg --cleanbuild --syncdeps` as an unprivileged user, run `namcap` on the recipe and package, and install the package. Confirm the installed `ancs-bridge version` returns `{"apiVersion":1,"version":"0.2.0"}`.
- [ ] Install that exact package on the physical Arch machine. With a real iPhone, verify notification delivery before and after reconnect, including the `notify_io()` path used when BlueZ reports `ServicesResolved=false`. If that path regresses, diagnose before publishing; consider backporting only the upstream compiler fix.
- [ ] Document the clean rolling Arch build/install check as a required pre-publication step for future Arch package releases.

## Out of scope

- Scheduled CI and a Rust beta matrix.
