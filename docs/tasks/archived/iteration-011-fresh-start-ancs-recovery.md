# Iteration 011 — Fresh-start ANCS recovery

**Status:** Completed

## Sources

- `docs/prd/01-daemon-core.md`
- `docs/prd/04-validation-release.md`
- `docs/specs/ancs-session-forwarding.md`
- `docs/specs/bluetooth-accessory-pairing.md`
- `docs/specs/setup-and-device-lifecycle.md`
- `docs/specs/systemd-user-service.md`
- `docs/tasks/archived/iteration-006-runtime-reliability-acceptance.md`

## Dependencies and guardrails

- The existing specifications already require setup-to-runtime cleanup and
  passive recovery to `ready` after every daemon start or restart, so this
  iteration fixes implementation gaps without changing specified behavior.
- Preserve passive iPhone-initiated reconnection. Recovery must not call generic
  `Device1.Connect()`, toggle adapter power, remove or replace bonds, change the
  configured identity, or require repeated iPhone Settings interaction.
- Preserve machine API v1 and the metadata-only privacy boundary. Diagnostic
  improvements may include error stage, stable code, timing, and BlueZ/D-Bus
  error metadata, but never notification payload values or secret data.
- The immutable `v0.1.0` release predates this fix. Do not complete the remaining
  package lifecycle or AUR publication work in iteration 008 with a known
  reconnect blocker; choose a new release version after this iteration is
  validated.

## Tasks

### Regression coverage and observability

- [x] Extend the fake transport and clock coverage to reproduce a fresh daemon
  remaining in `waiting-for-phone` without a previously active session, repeated
  startup subscription failure, and short transient waits that must not cause
  registration churn.
- [x] Add deterministic assertions for bounded passive refresh, retry ordering,
  backoff reset after success, and the absence of generic connect, adapter-power,
  bond, configuration, or device-selection mutations.
- [x] Retain and journal the metadata-only error chain and failed ANCS
  subscription stage instead of discarding the underlying error; record each
  deliberate BlueZ session/registration refresh with enough metadata to
  distinguish startup-stall, disconnect, and subscription-failure recovery.

### Cleanup and passive recovery

- [x] Introduce one bounded, cancellation-safe ANCS subscription quiescence path
  for partial subscription failure, ordinary session end, daemon shutdown, and
  setup cleanup, including a deterministic timeout fallback that cannot leave
  recovery permanently stalled.
- [x] Make setup wait for its temporary ANCS subscriptions and BlueZ-owned
  application/advertisement state to quiesce before completing the
  setup-to-runtime handoff, while preserving adapter restoration, transaction
  rollback, and error reporting.
- [x] Replace the current same-process `session_active` recovery gate with a
  bounded stall policy that can recreate the BlueZ session, GATT application,
  and runtime advertisement after fresh startup, repeated subscription failure,
  or disconnect, even when BlueZ's active-advertisement count still appears
  healthy.
- [x] Keep successful sessions stable and avoid continuous registration cycling:
  reset stall accounting after `ready`, tolerate ordinary short-lived
  connection, service-resolution, and authorization delays, and retain the
  existing BlueZ-unavailable backoff sequence.

### Automated and live validation

- [x] Extend the opt-in hardware acceptance harness with a bounded repeated
  service-restart stage that records metadata-only readiness timing and fails
  before any iPhone Bluetooth toggle or Settings intervention; preserve the
  configured identity, bonds, adapter state, audio intent, WirePlumber health,
  unrelated devices, and the no-generic-connect invariant on every cycle.
- [x] Run an existing-bond setup through successful ANCS verification and start
  the service immediately afterward; require passive return to `ready`, then
  complete at least twenty deliberate service restart cycles and forward one
  post-run notification exactly once without toggling iPhone Bluetooth.
- [x] Review the changed lifecycle code for avoidable complexity, apply only
  behavior-preserving simplifications, and run the relevant focused checks both
  before and after simplification.
- [x] Run formatting, Clippy with warnings denied, all non-hardware tests, the
  Linux dependency audit, locked release build, user-unit verification, and
  `git diff --check`; finish with live doctor checks passing and the enabled
  service running in `ready`.

## Implementation notes

- Added bounded ANCS notification quiescence shared by partial subscription
  failure, session end, BlueZ session reset, daemon shutdown, and setup cleanup.
- Added typed subscription failure stages and metadata-only journal fields for
  the underlying error chain, recovery cause, and bounded refresh attempt.
- Fresh startup and repeated subscription failures now recreate the passive
  BlueZ session after bounded thresholds; successful `ready` sessions reset the
  accounting and do not continuously cycle registrations.
- Updated the troubleshooting section to explain the passive recovery markers
  and metadata-only journal fields.
- Automated checks pass after the simplification review: `cargo fmt --check`,
  `cargo test --offline --locked --all-targets` (82 tests), Clippy with warnings
  denied, locked release build, the Linux-scoped RustSec audit with the two
  documented target-inactive exceptions, `systemd-analyze verify`, and
  `git diff --check`.
- The unrestricted `cargo audit` report contains the pre-existing
  `quick-xml 0.31.0` advisories `RUSTSEC-2026-0194` and `RUSTSEC-2026-0195`
  through the target-specific `notify-rust` Windows dependency. The Linux
  audit passes with exactly those two documented target-inactive exceptions;
  no broad audit suppression or unrelated dependency migration was made.
- Existing-bond setup, immediate service handoff, twenty deliberate service
  restarts, and one post-run notification canary passed in 275.70 seconds
  without an iPhone Bluetooth toggle or Settings intervention. The deployed
  daemon remains enabled, running, and `ready` with doctor checks passing.
