# Iteration 004 — Setup and device lifecycle

**Status:** Completed

## Sources

- `docs/prd/01-daemon-core.md`
- `docs/prd/02-machine-api.md`
- `docs/prd/03-packaging-security.md`
- `docs/prd/04-validation-release.md`
- `docs/specs/bluetooth-accessory-pairing.md`
- `docs/specs/setup-and-device-lifecycle.md`
- `docs/specs/phone-audio-suppression.md`
- `docs/specs/runtime-machine-api.md`
- `docs/tasks/archived/iteration-001-feasibility-report.md`
- `docs/tasks/archived/iteration-003-runtime-machine-api.md`

## Dependencies

- Iteration 003 provides validated configuration persistence and the stable read-only machine API.
- Automated behavior must use fake BlueZ, process, clock, filesystem, and service-control boundaries; only the explicit hardware tasks require the physical iPhone and audio devices.
- The disposable spike is evidence for BlueZ behavior, not production source.

## Tasks

### Machine contracts and command surface

- [x] Add stable Clap surfaces for `doctor --json`, `setup --jsonl [--disable-phone-audio] [--repair]`, and `teardown [--forget-device]` without changing the existing daemon/status/version contracts.
- [x] Define Serde types for doctor results and every setup JSONL command/event, rejecting unsupported versions, malformed types, wrong-state commands, unknown commands, and mismatched confirmation IDs before state changes.
- [x] Commit machine API v1 fixtures for every doctor status, setup state, pairing/existing-bond confirmation, completion, stable error code/recoverability pair, and malformed-input failure.
- [x] Add subprocess tests proving JSON/JSONL stdout framing and flushing, stderr separation, exit codes, stdin closure, cancellation, and unsupported API handling.

### Environment diagnostics

- [x] Add fakeable probes for BlueZ version, adapter inventory/power/roles/advertising capability, configured pairing, WirePlumber availability, and ANCS readiness.
- [x] Implement deterministic configured-or-only adapter selection with explicit zero-adapter and multiple-adapter failures and no adapter power mutation.
- [x] Implement the seven stable doctor checks with pass/warn/fail classification, top-level `ok`, BlueZ 5.87 validated-baseline reporting, and configuration-sensitive pairing/WirePlumber results.
- [x] Add doctor tests for supported, unconfigured, powered-off, missing/old BlueZ, ambiguous adapter, missing bond, disconnected phone, unavailable ANCS, optional WirePlumber, and required WirePlumber cases.

### Setup protocol engine

- [x] Implement the injectable setup state machine with five-minute candidate, 30-second confirmation, and 60-second ANCS-readiness deadlines.
- [x] Enforce one active opaque confirmation and exact acceptance of either a fresh pairing identity or a unique ANCS-ready existing bond, rejecting every competing device while pending.
- [x] Implement existing-bond reuse after caller confirmation and `repair-required` behavior that preserves bond/configuration without `--repair`.
- [x] Implement `--repair` for only the exact configured identity, with `repair-target-unknown` when no safe target exists and preservation of the old configuration until commit.
- [x] Add deterministic protocol/state tests for fresh success, existing-bond reuse, explicit repair, rejection, wrong device, each timeout, cancel at every state, malformed input, stdin closure, and unexpected backend failure.

### Production BlueZ setup transport

- [x] Add the discoverable/connectable setup advertisement using the production encrypted HID application and ANCS solicitation without any keyboard-report emission path.
- [x] Implement a temporary `DisplayYesNo`/`KeyboardDisplay` agent that forwards identity/passkey requests to the protocol engine and rejects unconfirmed or competing requests.
- [x] Capture adapter pairable/discoverable values, enable them only for the pairing window, and restore them on success, rejection, timeout, cancellation, stdin closure, SIGINT, SIGTERM, and returned errors.
- [x] Detect fresh paired candidates against the initial bond set, verify the confirmed identity is paired, mark it trusted, and never call generic `Device1.Connect()`.
- [x] Verify complete ANCS readiness within the bounded deadline, including Data Source before Notification Source subscription, then release temporary subscriptions and registrations.
- [x] Add fake transport/RAII tests proving cleanup ordering and no surviving agent, advertisement, GATT application, subscription, or adapter-state mutation on every exit path.

### Exact-device audio suppression and setup commit

- [x] Implement validated canonical WirePlumber path/rule generation with the exact device match, atomic `0600` creation, private missing directories, and conflict preservation.
- [x] Add fakeable user-service control and restart WirePlumber only after a real rule change, treating absence as optional unless suppression was requested.
- [x] Implement idempotent application/removal plus setup rollback that removes only a newly created rule and reloads WirePlumber when later commit fails.
- [x] Commit golden rule fixtures and tests for address normalization, invalid/path-like input, exact matching, repeated operations, conflicting content, restart failure, and rollback failure.
- [x] Orchestrate setup so temporary BlueZ cleanup and adapter restoration precede optional audio application, atomic configuration is last, and `complete` is emitted only after the full transaction commits.

### Teardown

- [x] Implement fakeable user-service stop/disable behavior that treats an absent `ancs-bridge.service` as success and emits no stdout.
- [x] Implement ordered, retry-safe teardown for the exact configured rule, optional exact configured BlueZ bond, and configuration-last deletion without wildcard cleanup.
- [x] Add teardown tests for no configuration, retained bond, forgotten bond, already absent resources, invalid configuration, rule conflict, and failures at each step proving configuration remains available for retry.

### Validation and documentation

- [x] Run the hardware setup matrix for fresh pairing, existing-bond reuse, explicit repair, rejection, cancellation, timeout/retry, wrong-device rejection, and ANCS readiness while recording metadata-only evidence.
- [x] Validate audio suppression on the configured iPhone while ANCS remains ready, and verify AirPods playback and microphone before, during, and after rule application/removal.
- [x] Validate teardown once retaining the bond and once forgetting it, confirming configuration/rule cleanup, WirePlumber health, and restored adapter/temporary-object state.
- [x] Update operator/developer documentation for doctor, JSONL orchestration, timeout/error behavior, manual SIGKILL recovery, audio-rule ownership, and teardown.
- [x] Run formatting, Clippy with warnings denied, all automated tests, the Linux-relevant dependency audit, and a locked release build; record continuing upstream compatibility exceptions.

## Implementation notes

- Automated validation passes with 70 non-hardware tests; the one opt-in hardware smoke remains ignored unless its adapter/device environment is supplied.
- The Linux dependency tree still excludes `quick-xml`; the RustSec audit retains the two documented Windows-only ignores.
- `bluer 0.17.3` retains its documented upstream future-compatibility warning. The locked Rust 2021 release build succeeds.
- Hardware setup evidence on 2026-08-19 covers fresh pairing, existing-bond reuse, rejection, cancellation, confirmation timeout/retry, ANCS readiness, stable identity resolution across an iOS privacy address, and restoration of adapter state.
- Hardware evidence on 2026-08-20 confirms explicit repair of only the configured iPhone, preservation of configuration through a timed-out attempt, successful retry and ANCS commit, rejection of a competing iPad with no resulting bond, and restoration of adapter/temporary-object state.
- Audio evidence on 2026-08-19 confirms the exact-device WirePlumber rule, continuing ANCS readiness, and working AirPods playback/microphone during suppression and after removal.
- Teardown evidence on 2026-08-19 covers both retained- and forgotten-bond modes, configuration/rule cleanup, healthy WirePlumber, preservation of unrelated bonds, and restored adapter/temporary-object state.

## Deferred work

- systemd user-unit installation and hardening, service enablement after setup, AUR packaging, `.SRCINFO`, and clean Arch package tests
- the wider restart/suspend/range-loss hardware matrix, twenty reconnect cycles, and release 0.1.0
