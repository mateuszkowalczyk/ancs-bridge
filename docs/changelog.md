# Changelog

## 2026-08-20 — Iteration 006

Iteration: `docs/tasks/archived/iteration-006-runtime-reliability-acceptance.md`

- Added a resumable, metadata-only hardware acceptance suite covering notification previews and lifecycle, privacy, service and Bluetooth disruptions, suspend recovery, and reconnect endurance.
- Hardened passive recovery across daemon and BlueZ restarts, lost registrations, active-phone disconnects, and suspend without generic Bluetooth connection attempts.
- Added forward compatibility for reserved iOS 26.6 ANCS flags and categories, and validated exact-device iPhone audio suppression with ANCS readiness and working AirPods playback and microphone.

## 2026-08-20 — Iteration 005

Iteration: `docs/tasks/archived/iteration-005-systemd-user-service.md`

- Added a hardened systemd user service with explicit activation, automatic login startup, private runtime state, and restart-on-failure recovery.
- Added reproducible source-install staging, exact unit/artifact validation, and reversible manual installation and service lifecycle documentation.
- Validated service-managed iPhone notification forwarding, deliberate lifecycle operations, configuration and bond preservation, and payload-free journal/status evidence.

## 2026-08-20 — Iteration 004

Iteration: `docs/tasks/archived/iteration-004-setup-device-lifecycle.md`

- Added stable machine-readable diagnostics and transactional iPhone setup with fresh pairing, existing-bond reuse, explicit repair, bounded confirmation, ANCS verification, and competing-device rejection.
- Added optional exact-device iPhone audio suppression that preserves ANCS and unrelated AirPods playback and microphone behavior.
- Added retry-safe teardown that can retain or forget only the configured bond while removing bridge-owned configuration and audio rules.

## 2026-08-19 — Iteration 003

Iteration: `docs/tasks/archived/iteration-003-runtime-machine-api.md`

- Added a config-driven daemon with validated version 1 TOML configuration and atomic owner-only configuration and runtime-status persistence.
- Added stable `version --json` and `status --json` machine commands, including live/stale daemon detection and useful unconfigured or stopped-daemon states.
- Added committed machine API v1 fixtures and privacy-focused command tests that keep notification content out of persistent and diagnostic output.

## 2026-08-19 — Iteration 002

Iteration: `docs/tasks/archived/iteration-002-production-core.md`

- Added the production ANCS bridge core with the encrypted HID accessory, bounded protocol decoding, session-scoped notification lifecycle, and Freedesktop delivery.
- Added automatic BlueZ, adapter, authorization, and iPhone disconnect recovery with passive bonded-device reconnection and no generic connection loop.
- Validated notification forwarding across an iPhone Bluetooth off/on cycle while keeping notification payload out of status and diagnostic evidence.

## 2026-08-19 — Iteration 001

Iteration: `docs/tasks/archived/iteration-001-ancs-feasibility-spike.md`

- Confirmed ANCS feasibility on the target Intel/BlueZ environment with fresh iPhone pairing and Freedesktop notification delivery.
- Demonstrated existing-bond restart and automatic disconnect/reconnect recovery without generic Bluetooth connection attempts or notification-payload persistence.
