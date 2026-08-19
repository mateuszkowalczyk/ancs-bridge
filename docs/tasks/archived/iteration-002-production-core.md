# Iteration 002 — Production core and recovery supervisor

**Status:** Completed

## Sources

- `docs/prd/01-daemon-core.md`
- `docs/prd/02-machine-api.md`
- `docs/prd/04-validation-release.md`
- `docs/specs/bluetooth-accessory-pairing.md`
- `docs/specs/ancs-session-forwarding.md`
- `docs/tasks/archived/iteration-001-feasibility-report.md`

## Dependencies

- Iteration 001 recorded a **Go** decision for the validated Intel/BlueZ/iPhone environment.
- Automated behavior must remain testable without Bluetooth hardware; the physical iPhone is needed only for the explicit hardware smoke tasks.
- The disposable `spike/` may be consulted as evidence but must not be promoted, imported, or copied wholesale into the production architecture.

## Tasks

### Production crate foundation

- [x] Create the root `ancs-bridge` Rust package from scratch with the approved Tokio, `bluer`, Clap, Serde, TOML, `notify-rust`, and tracing stack; commit `Cargo.lock`, an MIT license, and no dependency on the `ancs` crate.
- [x] Establish production module boundaries for Bluetooth transport/supervision, local HID accessory, ANCS codec/session, notification delivery, clock, and status publication without importing the disposable spike crate.
- [x] Define fakeable `BluetoothTransport`, `NotificationSink`, `Clock`, and `StatusWriter` boundaries plus deterministic test doubles for each.
- [x] Separate notification payload values from diagnostic metadata so tracing and status APIs cannot accidentally serialize app identifiers, titles, messages, or other payload content.

### Runtime HID accessory

- [x] Implement the production HID-over-GATT service with HID Information, Report Map, Control Point, Protocol Mode, input Report, and Report Reference descriptor using the encryption requirements from the pairing spec.
- [x] Implement the connectable, non-discoverable runtime advertisement with the HID service UUID and ANCS service-solicitation UUID without changing adapter power.
- [x] Keep GATT application and advertisement registrations alive through explicit RAII/session handles and prove cleanup when those handles or the BlueZ connection are dropped.
- [x] Add tests that verify the complete HID shape, encrypted read/write flags, solicitation data, runtime discoverability policy, and absence of any keyboard-report emission path.

### ANCS codec

- [x] Implement local protocol types and strict parsing for all used eight-byte Added, Modified, and Removed Notification Source events, flags, categories, and little-endian UIDs.
- [x] Implement Get Notification Attributes and Get App Attributes encoders with title and message limits, including an explicit write-with-response requirement for Control Point transport calls.
- [x] Implement incremental Data Source response decoding across arbitrary fragment boundaries and combined responses with command, UID, attribute-order, length, UTF-8, and 64 KiB cap validation.
- [x] Add golden vectors for every used command, event, and attribute structure plus fragmentation at every byte boundary and multiple responses in combined chunks.
- [x] Add negative tests for truncated, oversized, malformed, unknown-command, wrong-UID, invalid-attribute, invalid-length, and invalid-UTF-8 input, proving that none can panic or grow memory without bound.

### ANCS session engine and notification lifecycle

- [x] Implement a scheduler with exactly one outstanding Control Point request, a five-second injected-clock timeout, and at most 100 queued/pending notification UIDs.
- [x] Implement Modified coalescing, Removed cancellation/close behavior, and `PreExisting` suppression without issuing stale attribute requests.
- [x] Implement session-scoped app display-name caching and bundle-identifier fallback after app lookup failure or timeout.
- [x] Implement Added/create, Modified/replace, and Removed/close behavior through `NotificationSink`, with delivery failures recorded as metadata-only recoverable errors.
- [x] Clear subscriptions, outstanding/queued requests, UIDs, notification handles, and app-name cache at session end without retaining payload content.
- [x] Add deterministic session tests for serialization, timeout/recovery, queue bounds, cancellation, coalescing, cache/fallback behavior, notification lifecycle, delivery failure, and privacy canaries.
- [x] Implement the production Freedesktop sink with `notify-rust` while keeping all action/reply behavior absent from V1.

### BlueZ supervisor and recovery

- [x] Implement the production `bluer` transport for system-bus/adapter discovery, runtime GATT and advertisement registration, configured bonded-device selection, ANCS characteristic discovery, ordered subscriptions, and explicit `WriteOp::Request` Control Point writes.
- [x] Implement supervisor states and status-writer updates for BlueZ/adapter wait, advertising, phone wait, service wait, authorization wait, subscribing, ready, backoff, and error transitions.
- [x] Treat delayed, disappearing, or reauthorized ANCS as recoverable while connected, and reconcile adapter, registrations, device, services, and subscriptions every five seconds.
- [x] On phone disconnect, end the ANCS session and wait for an incoming bonded-device reconnect without any generic `Device1.Connect()` recovery loop.
- [x] On BlueZ restart or adapter loss, drop invalid handles and retry/recreate the full BlueZ session after 1, 2, 5, 10, and then 30 seconds, resetting backoff after successful re-establishment.
- [x] Add fake BlueZ/clock/status tests for delayed authorization, service disappearance/reappearance, phone disconnect/reconnect, BlueZ restart, adapter loss, suspend-style missed events recovered by reconciliation, transition reporting, and backoff reset.

### Validation and evidence

- [x] Add an ignored, explicit opt-in hardware smoke harness that accepts the adapter and bonded iPhone identity as runtime inputs and exercises only the production modules.
- [x] Run the production smoke harness on the validated hardware to prove `ready`, one forwarded notification, and Bluetooth off/on recovery without device selection or `Device1.Connect()`, recording metadata-only evidence.
- [x] Run `cargo fmt --check`, Clippy with warnings denied, the full automated test suite, dependency audit, and a locked release build; document the known `bluer` future-compatibility status.
- [x] Update production developer documentation with module responsibilities, fake/hardware test commands, privacy constraints, and the rule that the spike is evidence rather than production source.

## Hardware evidence

- 2026-08-19, `hci0`, configured bonded iPhone: production modules reached `ready` with Data Source and Notification Source subscribed, then forwarded one notification (`deliveredCount=1`).
- Turning iPhone Bluetooth off was observed as `waiting-for-phone`; the active ANCS session was cleared and the supervisor continued passive waiting without device selection or `Device1.Connect()`.
- Turning Bluetooth back on produced passive `waiting-for-services` → `subscribing` → `ready` recovery. A second notification was forwarded in the new ANCS session (`deliveredCount=2`).
- The ignored production smoke test passed in 157.09 seconds with `passiveReconnect=true` and `payloadLogged=false`; evidence contained only states, booleans, reason codes, and counters.

## Completion notes

- All 30 iteration tasks are complete.
- Automated tests, formatting, Clippy with warnings denied, the Linux-relevant RustSec audit, and the locked release build pass.
- `bluer` 0.17.3 still emits the documented upstream Rust future-compatibility warning.

## Deferred work

- Stable CLI commands, configuration/status persistence, machine API fixtures, and setup JSONL behavior.
- Production pairing agent, existing-bond/re-pair workflow, exact-device WirePlumber suppression, and teardown.
- systemd user unit, AUR packaging, clean-environment packaging tests, full hardware matrix, and release 0.1.0.
