# systemd user service

## Purpose

Run the configured bridge automatically in the logged-in user's session while
preserving the foreground CLI, explicit setup authorization, privacy boundary,
and retry-safe teardown behavior.

## Unit contract

The repository ships `ancs-bridge.service` for installation at:

```text
/usr/lib/systemd/user/ancs-bridge.service
```

The service runs `/usr/bin/ancs-bridge daemon` as the logged-in user. It uses:

- `Restart=on-failure`
- `RestartSec=3`
- `RuntimeDirectory=ancs-bridge`
- `UMask=0077`
- `WantedBy=default.target`

It includes `NoNewPrivileges=true` and `PrivateTmp=true`. Additional hardening
is included only when live validation proves that BlueZ and desktop
notification D-Bus access, runtime-status publication, and restart recovery
remain functional. The unit does not require root, sudo, an Omarchy runtime,
or network access.

## Installation and activation policy

A package or source-install step may install the binary, license, and user
unit, but never enables or starts the service, pairs a device, changes user
configuration, or restarts WirePlumber.

Setup remains a transactional configuration command and does not silently
change service enablement. After setup succeeds, the user or an authorized
frontend explicitly activates automatic operation with:

```text
systemctl --user enable --now ancs-bridge.service
```

Enabling before valid setup is unsupported operator ordering. The foreground
`ancs-bridge daemon` command remains available without enabling the unit.

Until source-built packaging is delivered, the developer documentation gives
explicit, reversible commands for building and manually installing exactly the
binary, license, and unit to their final system paths. It separately documents
`daemon-reload`, enable/start, status inspection, logs, disable/stop, and manual
removal. Manual removal does not delete configuration or the Bluetooth bond;
the user runs `ancs-bridge teardown` first when that cleanup is desired.

## Runtime lifecycle

Service startup loads the existing validated configuration and recreates the
runtime directory, status file, BlueZ application, and advertisement. A
missing or invalid configuration fails visibly rather than selecting or
mutating a device.

An unexpected daemon failure is restarted after three seconds. A deliberate
service stop is not restarted. Restart and disable/enable cycles preserve
configuration, the exact iPhone bond, and any configured audio-suppression
intent. After each start or restart, the supervisor reaches `ready` without a
generic Bluetooth connection loop or repeated iPhone Settings interaction.

`ancs-bridge teardown` continues to stop and disable the installed unit before
removing other bridge-owned state. An absent unit remains a successful no-op.

## Status, logs, and privacy

Operators use `systemctl --user status`, `journalctl --user-unit`, and
`ancs-bridge status --json` to inspect the service. Journaling contains only
process diagnostics and metadata already allowed by the privacy requirements.
Notification titles, bodies, app payloads, and secret values never appear in
the unit, environment, journal, runtime status, or installation artifacts.

## Acceptance criteria

- The checked-in unit passes `systemd-analyze verify` and contains the required
  executable path, restart policy, runtime-directory policy, permissions,
  install target, and compatible hardening.
- A staged source installation contains only the binary, license, and user
  unit at the documented final paths and performs no automatic user-state
  mutation.
- After explicit enable/start, the service reaches `ready` and forwards a
  notification through the production daemon.
- An injected process failure produces a new daemon process that returns to
  `ready`; deliberate stop/start and disable/enable behave predictably.
- Configuration, pairing, audio intent, runtime status, and unrelated
  Bluetooth devices survive service lifecycle operations.
- Service diagnostics and journal evidence contain no notification payload.
- Source installation and activation documentation is complete and reversible
  without claiming that AUR packaging or release validation is finished.
