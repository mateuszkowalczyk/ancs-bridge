# Specification: `ancs-bridge` Machine API v1

This is the canonical contract consumed by external frontends, including `omarchy-iphone-notifications`. Breaking changes require a new machine API version. Additive fields may be introduced within version 1 and consumers must ignore unknown fields.

## CLI

Install `/usr/bin/ancs-bridge` with:

- `ancs-bridge daemon`: long-running bridge used by systemd.
- `ancs-bridge setup --jsonl --disable-phone-audio`: interactive setup/pairing protocol.
- `ancs-bridge doctor --json`: checks BlueZ version, adapter central/peripheral roles, LE advertising, existing pairing, WirePlumber availability, and ANCS readiness.
- `ancs-bridge status --json`: returns current runtime state and remains useful when the status file is stale.
- `ancs-bridge teardown [--forget-device]`: removes bridge configuration and its exact-device audio rule; optionally forgets the phone.
- `ancs-bridge version --json`: returns semantic version and machine API version.

JSON-producing commands return nonzero on failure. Stdout is reserved for machine-readable JSON/JSONL; human diagnostics go to stderr.

## Configuration schema

Write atomically with mode `0600` to `~/.config/ancs-bridge/config.toml`:

```toml
schema_version = 1

[bluetooth]
adapter = "hci0"
device_address = "AA:BB:CC:DD:EE:FF"
device_name = "iPhone"

[desktop]
suppress_phone_audio = true
```

The Bluetooth address is the bonded BlueZ device identity selected during setup. Validate it before persistence or path generation.

## Runtime status schema

Write atomically to `$XDG_RUNTIME_DIR/ancs-bridge/status.json`:

```json
{
  "apiVersion": 1,
  "state": "ready",
  "reasonCode": null,
  "adapter": "hci0",
  "deviceAddress": "AA:BB:CC:DD:EE:FF",
  "deviceName": "iPhone",
  "connected": true,
  "servicesResolved": true,
  "ancsAvailable": true,
  "subscribed": true,
  "lastErrorCode": null,
  "lastTransitionAt": "RFC3339 timestamp",
  "lastNotificationAt": "RFC3339 timestamp",
  "pid": 1234
}
```

No notification title, body, app payload, or other notification content may appear in this file.

Valid states:

- `unconfigured`
- `waiting-for-bluez`
- `waiting-for-adapter`
- `advertising`
- `waiting-for-phone`
- `waiting-for-services`
- `waiting-for-authorization`
- `subscribing`
- `ready`
- `backoff`
- `error`

`reasonCode` and `lastErrorCode` are stable machine codes, not localized prose. Timestamps are RFC 3339 strings or null when the event has never occurred.

## Setup JSON Lines protocol

`ancs-bridge setup --jsonl --disable-phone-audio` reads one JSON object per stdin line and writes one JSON object per stdout line.

Required daemon events include:

```json
{"v":1,"event":"state","state":"waiting-for-iphone"}
{"v":1,"event":"confirmation-request","requestId":"opaque-id","deviceName":"iPhone","address":"AA:BB:CC:DD:EE:FF","passkey":"123456"}
{"v":1,"event":"complete","address":"AA:BB:CC:DD:EE:FF"}
{"v":1,"event":"error","code":"stable-error-code","recoverable":true}
```

Required caller commands include:

```json
{"v":1,"command":"confirm","requestId":"opaque-id","accept":true}
{"v":1,"command":"cancel"}
```

Protocol requirements:

- Only one confirmation request may be active.
- Confirmation IDs are opaque and must match exactly.
- Confirmation contains the incoming device name/address and six-digit passkey when BlueZ supplies one.
- Reject, cancel, timeout, stdin closure, or process failure must not persist partial configuration.
- `complete` is emitted only after confirmed pairing/trust, atomic configuration write, requested audio-rule application, and temporary adapter-state restoration succeed.
- `error.code` is stable and `recoverable` tells the caller whether retrying the current workflow is meaningful.
- Malformed input returns an error event or nonzero exit without executing unintended commands.

## Compatibility rules

- `version --json`, `doctor --json`, `status --json`, runtime status, and setup JSONL identify API version 1.
- Consumers ignore unknown additive object fields.
- Consumers must reject unsupported API versions and malformed required fields before invoking state-changing commands.
- Semantic package versions may advance without an API change; a breaking machine-interface change requires API version 2.
- Command names, required fields, status-state meanings, and JSONL command/event semantics are stable for API version 1.

