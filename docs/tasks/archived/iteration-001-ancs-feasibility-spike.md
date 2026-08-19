# Iteration 001 — ANCS feasibility spike

**Status:** Completed

## Sources

- `docs/prd/01-daemon-core.md`
- `docs/prd/04-validation-release.md`

## Dependencies

- A target Intel Bluetooth adapter running BlueZ 5.87 or the closest available test environment.
- A physical iPhone that can be freshly paired from iOS Bluetooth settings.
- An active Freedesktop notification session for the end-to-end check.

## Tasks

### Spike foundation

- [x] Record the test hardware, BlueZ version, iPhone/iOS version, adapter capabilities, and any material differences from the target environment.
- [x] Create an explicitly disposable Rust spike with only the dependencies and diagnostics needed to exercise BlueZ, ANCS, and Freedesktop notifications.
- [x] Ensure every exit path attempted during the spike unregisters temporary BlueZ objects and restores adapter pairable/discoverable settings.

### Bluetooth accessory and pairing

- [x] Register the minimal HID-over-GATT keyboard service required by the PRD without sending input reports or keyboard events.
- [x] Advertise the HID service together with the ANCS service-solicitation UUID in a discoverable/connectable setup advertisement.
- [x] Register a temporary BlueZ pairing agent and complete a fresh pairing initiated from the iPhone Bluetooth settings.
- [x] Capture evidence that the paired iPhone connects, resolves services, and exposes ANCS without patched BlueZ or iOS.

### ANCS end-to-end proof

- [x] Discover the ANCS Notification Source, Data Source, and Control Point characteristics after authorization.
- [x] Subscribe to Data Source before Notification Source and capture one valid notification event.
- [x] Request and minimally decode the attributes needed to render that notification, while rejecting malformed or oversized spike input without panicking.
- [x] Forward the captured notification through the standard Freedesktop notification service and verify that its content is not written to persistent files or logs.

### Lifecycle and feasibility decision

- [x] Exercise daemon restart and at least one phone disconnect/reconnect cycle, recording whether ANCS authorization and delivery recover without repeated generic `Device1.Connect()` calls or repeated iPhone Settings interaction.
- [x] Document the spike procedure, observed evidence, limitations, and cleanup steps in a reproducible feasibility report.
- [x] Record a go, redesign, or stop decision against every Phase 0 blocker in `docs/prd/04-validation-release.md`, including any PRD or spec follow-up that must be approved before production work.
- [x] Remove or clearly isolate disposable spike artifacts so they cannot be mistaken for the production daemon architecture.

## Implementation notes

- The 2026-08-19 host run confirmed that BlueZ 5.87 accepted the local GATT application and advertisement on `hci0`.
- Cancelling the waiting fresh-pair run with Ctrl-C restored Pairable/Discoverable to their captured values and returned LE advertising `ActiveInstances` to zero.
- Fresh pairing succeeded with encrypted HID attributes and exact-device automatic confirmation; other devices remained rejected.
- iOS published ANCS shortly after pairing. The spike now waits for that delayed publication rather than treating the first service set as final.
- ANCS Control Point writes must explicitly use `WriteOp::Request`; `bluer` defaults to a write-without-response command.
- Existing-bond restart and automatic reconnect after iPhone Bluetooth off/on both resubscribed and delivered without `Device1.Connect()` or repeated device selection.

## Deferred work

- Production daemon architecture, full ANCS codec, recovery supervisor, and comprehensive automated tests.
- Machine API v1, production setup workflow, WirePlumber integration, systemd service, AUR packaging, and release validation.
