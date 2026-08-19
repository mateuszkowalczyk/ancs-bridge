# PRD: `ancs-bridge` Daemon Core

## Objective

Present the minimal Bluetooth LE accessory shape needed for an iPhone to expose ANCS, translate ANCS events into Freedesktop desktop notifications, and survive normal Bluetooth and system lifecycle changes without an Omarchy dependency.

## Product and repository boundaries

Build a distribution-neutral, MIT-licensed Rust daemon that owns the Bluetooth/ANCS implementation, versioned machine API, systemd user service, and source-built AUR package. It must remain usable without Omarchy or Quickshell; graphical frontends are consumers of its public CLI and machine API, not runtime dependencies.

Implement the daemon from scratch. Do not fork or reuse the existing local `ancsd` or `ancs-linux` checkout. [`ancs-linux`](https://github.com/kmod-midori/ancs-linux) is prior art only because its prototype structure lacks the lifecycle, stable API, packaging, recovery model, and tests required here. Implement the narrow codec locally from [Apple's ANCS specification](https://developer.apple.com/library/archive/documentation/CoreBluetooth/Reference/AppleNotificationCenterServiceSpecification/Specification/Specification.html) using [`bluer`](https://docs.rs/bluer/latest/bluer/) for BlueZ. If any prior-art code is eventually copied, retain its MIT attribution.

V1 must provide:

- One configured iPhone.
- Read-only forwarding with Added, Modified, and Removed notification lifecycle behavior.
- Interactive setup suitable for a graphical caller, plus standalone diagnostics, status, and teardown commands.
- Automatic operation at login and recovery after ordinary disconnects, suspend/resume, adapter loss, and BlueZ restart.
- Optional suppression of only the configured iPhone as a PipeWire audio device.

V1 explicitly excludes notification actions, replies, history, filters, multiple phones, media control, Apple Media Service, MAP/SMS, and Omarchy-specific frontend behavior.

## Implementation architecture

Use Rust with Tokio, [`bluer`](https://docs.rs/bluer/latest/bluer/), Clap, Serde/Serde JSON, TOML, `notify-rust`, tracing, and only small necessary supporting crates. Commit `Cargo.lock`. Do not depend on the lightly maintained Rust `ancs` crate.

Separate the implementation into:

- CLI, configuration, and atomic status publishing.
- BlueZ supervisor and adapter/device lifecycle.
- Temporary setup agent.
- Minimal HID-over-GATT application and advertisement.
- ANCS codec and per-phone session.
- Freedesktop notification sink.
- Exact-device WirePlumber audio suppression.

Abstract BlueZ transport, notification sink, clock, and status writer behind traits so the state machine is testable without hardware.

## Bluetooth accessory behavior

- Register a minimal HID-over-GATT keyboard shape with HID Information, Report Map, Control Point, Protocol Mode, Report, and Report Reference descriptor.
- Never send input reports or keyboard events.
- Advertise the HID service and ANCS service-solicitation UUID.
- Use a discoverable/connectable advertisement during setup and a connectable, non-discoverable advertisement at runtime.
- Keep the GATT application and advertisement handles alive for the process lifetime.
- During runtime, accept only the configured bonded phone wherever BlueZ APIs permit filtering.
- Never force Bluetooth power; that is the caller's policy.
- Never use repeated generic `Device1.Connect()` as routine reconnection. BlueZ's generic connect attempts available profiles and chooses a bearer, making it unsuitable as the recovery loop ([BlueZ Device API](https://github.com/bluez/bluez/blob/master/doc/org.bluez.Device.rst)).

## ANCS protocol and session

Implement the codec directly from [Apple's ANCS specification](https://developer.apple.com/library/archive/documentation/CoreBluetooth/Reference/AppleNotificationCenterServiceSpecification/Specification/Specification.html):

- Discover Notification Source, Data Source, and Control Point after `ServicesResolved` and ANCS authorization.
- Subscribe to Data Source before Notification Source.
- Parse the eight-byte notification event structure.
- Allow exactly one outstanding Control Point request, with a five-second timeout and bounded pending queue.
- Incrementally reassemble fragmented Data Source responses with a 64 KiB hard cap.
- Validate command IDs, attribute IDs, lengths, and UTF-8; malformed input must never panic.
- Request app identifier, app display name, title up to 256 bytes, and message up to 2048 bytes.
- Maintain at most 100 pending notification UIDs.
- Coalesce Modified events by UID, cancel pending work on Removed, and skip `PreExisting` notifications to avoid a login flood.
- Cache app display names in memory for the active ANCS session and fall back to bundle identifier after timeout.
- Treat malformed packets, invalid UTF-8, disappearing services, and notification-service failures as recoverable.

## Desktop notification behavior

- Deliver through the standard Freedesktop notification service using `notify-rust`.
- Map each ANCS UID to its desktop notification handle for the active session.
- Added creates a notification, Modified replaces it, and Removed closes it.
- V1 exposes no action buttons and sends no commands back to the phone.
- Notification-delivery failure logs metadata only, optionally updates diagnostic state, and does not terminate the Bluetooth session.
- Notification content remains in memory only long enough to deliver the desktop notification.

## Runtime state machine

At daemon start:

1. Connect to BlueZ and locate the configured adapter.
2. Register the HID GATT application and runtime advertisement.
3. Accept only the configured bonded phone.
4. Wait for the iPhone to initiate or restore the connection.
5. Wait for `ServicesResolved` and ANCS authorization.
6. Subscribe Data Source and then Notification Source.
7. Publish `ready`.

On phone disconnect:

- Cancel subscriptions and pending requests.
- Close or clear session notification handles and app-name cache.
- Keep or recreate the advertisement as required.
- Return to `waiting-for-phone`.

On BlueZ restart or adapter loss:

- Drop invalid D-Bus, application, advertisement, device, and characteristic handles.
- Retry after 1, 2, 5, 10, and then 30 seconds.
- Recreate the BlueZ session and registrations after recovery.

Reconcile adapter, advertisement, paired-device, service, and subscription state every five seconds. This must recover after suspend/lid-open without a dedicated lid hook.

If the phone is connected but ANCS is unavailable or unauthorized, publish `waiting-for-authorization` and retry while connected. Tolerate ANCS appearing, disappearing, and being reauthorized during a connection.

## Setup implementation responsibilities

When invoked in setup mode, the daemon must:

1. Capture the adapter's pairable/discoverable settings.
2. Register the HID GATT application and setup advertisement before pairing.
3. Register a temporary `DisplayYesNo`/`KeyboardDisplay` BlueZ agent.
4. Wait for pairing initiated from iPhone Bluetooth settings.
5. Ask the caller to confirm the incoming identity/passkey through the JSONL contract.
6. Accept, trust, and record only the confirmed device.
7. Verify bonding/pairing, then restore the previous adapter settings.
8. Write configuration atomically.
9. Apply exact-device WirePlumber suppression when requested.
10. Unregister temporary objects and exit so the production service can start.

Every success, rejection, cancellation, timeout, stdin closure, and unexpected-error path must restore temporary adapter settings and unregister the temporary agent/GATT objects.

An existing pairing may be reused only when diagnostics prove ANCS readiness. Otherwise setup requires explicit caller authorization before forgetting and re-pairing it.

## Acceptance criteria

- A configured phone reaches `ready` without Omarchy being installed.
- ANCS Added, Modified, and Removed events produce the corresponding desktop-notification lifecycle.
- BlueZ restart, adapter loss, phone disconnect, and suspend do not leave a permanently dead daemon.
- Routine recovery does not spam generic connection attempts.
- Malformed/fragmented ANCS data cannot panic or cause unbounded memory growth.
- No notification payload appears in persistent configuration, runtime status, or journal output.
