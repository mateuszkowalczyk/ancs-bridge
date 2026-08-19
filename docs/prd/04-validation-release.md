# PRD: Daemon Feasibility, Validation, and Release

## Implementation sequence

1. Complete the protocol and hardware feasibility spike.
2. Implement the ANCS codec, fakeable interfaces, and BlueZ supervisor.
3. Stabilize machine API version 1, including CLI, status, and setup JSONL fixtures.
4. Implement pairing setup, exact-device audio suppression, and teardown.
5. Package and validate the systemd user service and source-built AUR distribution.
6. Complete automated and hardware acceptance, then release `ancs-bridge` 0.1.0.

## Phase 0 feasibility spike

Before production architecture or plugin work, build a minimal Rust spike that:

- Registers a minimal HID-over-GATT keyboard service without emitting keyboard input.
- Advertises the HID service and ANCS service-solicitation UUID.
- Registers a temporary BlueZ pairing agent.
- Completes fresh pairing initiated from iPhone Bluetooth settings.
- Discovers ANCS Notification Source, Data Source, and Control Point.
- Receives and forwards one notification through the standard Linux notification system.

Stop or redesign before product implementation if pairing needs patched BlueZ/iOS, stable ANCS authorization cannot be achieved, routine reconnection needs repeated generic `Device1.Connect()`, or the HID technique is unreliable on the target Intel adapter. Discard spike-quality structure afterward; production code follows the modular daemon PRD.

## Automated daemon tests

- Golden ANCS vectors for every used event, command, and attribute structure.
- Fragmentation at every byte boundary and multiple responses in combined chunks.
- Truncated, oversized, malformed, invalid-command, invalid-attribute, invalid-length, and invalid-UTF-8 inputs.
- One-at-a-time Control Point serialization, five-second timeout, bounded pending queue, cancellation, and recovery.
- UID queue limit of 100, Modified coalescing, Removed cancellation, and `PreExisting` suppression.
- Mock notification sink tests for Added/create, Modified/replace, Removed/close, app-name cache, and bundle-ID fallback.
- Fake BlueZ/clock tests for phone disconnect, service disappearance/reappearance, delayed authorization, BlueZ restart, adapter loss, suspend-style reconciliation, and backoff reset.
- Setup JSONL confirmation, rejection, timeout, cancellation, malformed input, stdin closure, unexpected exit, and API incompatibility.
- Atomic configuration/status writes, mode `0600` and runtime permissions, status schema, and absence of notification content.
- Exact-MAC WirePlumber generation, idempotent application, reversal, and path/input validation.
- `cargo fmt --check`, Clippy with warnings denied, unit/integration tests, dependency audit, and clean release build.

## Packaging tests

- `makepkg --cleanbuild`, `namcap`, and `.SRCINFO` validation in clean Arch.
- Clean install, upgrade, service enable/start, service restart, stop/disable, and package removal.
- Verify installed artifacts are only binary, license, and user unit.
- Verify no install hook enables services or mutates user configuration.
- Inject canary notification content and prove it does not remain in status, files, diagnostics, or journal output.

## Hardware acceptance

Run on Omarchy 4 or equivalent current Arch userspace, BlueZ 5.87, the detected Intel controller, and a physical iPhone:

- Fresh pairing, existing-pair reuse when ANCS-ready, explicit re-pair, wrong-device rejection, passkey rejection, cancellation, timeout, and retry.
- Notifications from representative apps while locked/unlocked and under different preview settings.
- Added, Modified, and Removed notification behavior.
- Recovery after daemon restart, BlueZ restart, adapter power cycle, suspend/lid cycle, iPhone Bluetooth off/on, range loss, and computer reboot/login.
- Twenty disconnect/reconnect cycles without a dead session, unbounded growth, or unintended duplicate notifications.
- No notification content in journal, status, configuration, or setup diagnostics.
- Configured iPhone absent as a PipeWire audio device while ANCS remains `ready`.
- AirPods playback and microphone working before, during, and after setup/reconnection.
- Teardown retaining pairing and teardown forgetting pairing; WirePlumber remains healthy and the rule is removed.

Routine reconnect acceptance means the next notification is forwarded once `ready`, without desktop intervention or repeated iPhone Settings interaction. A one-time iPhone action is acceptable when iOS explicitly requires notification authorization or pairing.

## Release blockers

Do not release if:

- HID pairing cannot produce stable ANCS authorization.
- Routine reconnection depends on generic BlueZ `Connect()` or repeated user intervention.
- Audio suppression breaks ANCS/GATT or affects AirPods.
- Notification content leaks into journal, files, status, or diagnostics.
- BlueZ restart, suspend, or range loss can leave a permanently dead session.
- API incompatibility can trigger state-changing behavior.
- Setup cancellation leaves the adapter discoverable/pairable or stale GATT/agent registrations.

## Daemon rollout

1. Release a hardware alpha for the validated BlueZ 5.87/Intel/iPhone environment.
2. Resolve blockers and rerun the full daemon hardware matrix.
3. Publish `ancs-bridge` 0.1.0 as a source-built AUR package.
4. Treat other capable adapters as experimental until the same matrix passes.
