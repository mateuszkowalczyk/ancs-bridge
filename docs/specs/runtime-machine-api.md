# Runtime configuration and read-only machine API

## Purpose

Define the stable configuration, runtime-status, and read-only CLI behavior
needed to run and observe one configured `ancs-bridge` daemon. Setup, pairing,
diagnostics, audio suppression, and teardown remain outside this capability.

## Configuration

The configuration path is
`$XDG_CONFIG_HOME/ancs-bridge/config.toml`, falling back to
`~/.config/ancs-bridge/config.toml` when `XDG_CONFIG_HOME` is unset. It uses the
version 1 TOML schema from `docs/prd/02-machine-api.md`.

The loader rejects an unsupported schema version, a missing or empty adapter,
and an invalid Bluetooth identity address before constructing Bluetooth paths
or starting the supervisor. The persisted device name is display metadata;
the validated bonded-device identity address is the device key.

Configuration replacement is atomic and the resulting file has mode `0600`.
Failed writes do not replace the last valid configuration. Configuration never
contains notification payload data.

`ancs-bridge daemon` accepts no device-selection arguments. It loads the
configuration, starts the existing runtime supervisor for that exact adapter
and bonded device, and reports startup failure on stderr with a nonzero exit.
It writes no machine data to stdout.

## Runtime status publication

The daemon writes the version 1 status object defined in
`docs/prd/02-machine-api.md` to
`$XDG_RUNTIME_DIR/ancs-bridge/status.json`. The runtime directory has mode
`0700`, the status file has mode `0600`, and each replacement is atomic.

The status object contains only:

- API version, current state, and stable reason/error codes
- configured adapter and device display metadata
- connection, service, ANCS, and subscription booleans
- last-transition and last-notification timestamps
- daemon process ID

`lastTransitionAt` changes when the published state or current reason changes.
`lastNotificationAt` changes after a successful Added or Modified desktop
notification delivery. Timestamps are UTC RFC 3339 strings. Fields whose event
has never occurred are null.

The daemon publishes after startup and every state transition, and republishes
when notification or error metadata changes. A status-write failure is a
metadata-only recoverable error and cannot terminate the Bluetooth session.
Notification app identifiers, titles, messages, and other payload values never
appear in status files, status output, or status-write diagnostics.

## `status --json`

On success, `ancs-bridge status --json` writes exactly one version 1 status
object to stdout. It adds one field to the persisted schema:

```json
"stale": false
```

The command applies these rules:

- A valid snapshot belonging to a live `ancs-bridge` daemon and matching the
  current configuration returns `stale: false`.
- If the recorded daemon is no longer running, or the snapshot no longer
  matches the configuration, the last valid snapshot is preserved and returned
  with `stale: true`.
- With no configuration, the command synthesizes `state: "unconfigured"`,
  null adapter/device/timestamp/PID fields, false runtime booleans, and
  `stale: false`.
- With valid configuration but no status file, the command synthesizes
  `state: "error"`, `reasonCode: "daemon-not-running"`, the configured device
  metadata, null timestamps/PID, false runtime booleans, and `stale: true`.

A malformed configuration, malformed status document, unsupported schema/API
version, or unexpected filesystem failure produces no stdout, writes a concise
diagnostic to stderr, and exits nonzero.

## `version --json`

On success, `ancs-bridge version --json` writes exactly this shape to stdout,
followed by a newline:

```json
{"apiVersion":1,"version":"0.1.0"}
```

`version` is the package semantic version. Additional fields may be added under
the machine API v1 compatibility rules, but these two fields remain required.

## Acceptance criteria

- The daemon starts from one validated version 1 configuration without CLI
  adapter or device arguments.
- Configuration and status replacement is atomic and owner-only, including
  replacement of files that already exist with broader permissions.
- Runtime status reflects supervisor transitions and successful notification
  delivery without persisting notification payloads.
- `status --json` deterministically distinguishes live, stale, unconfigured,
  and configured-but-not-running cases using only the `stale` addition.
- `version --json` and every successful status result match committed machine
  API v1 fixtures; failures keep stdout empty and return nonzero.
