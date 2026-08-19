# Changelog

## 2026-08-19 — Iteration 002

Iteration: `docs/tasks/archived/iteration-002-production-core.md`

- Added the production ANCS bridge core with the encrypted HID accessory, bounded protocol decoding, session-scoped notification lifecycle, and Freedesktop delivery.
- Added automatic BlueZ, adapter, authorization, and iPhone disconnect recovery with passive bonded-device reconnection and no generic connection loop.
- Validated notification forwarding across an iPhone Bluetooth off/on cycle while keeping notification payload out of status and diagnostic evidence.

## 2026-08-19 — Iteration 001

Iteration: `docs/tasks/archived/iteration-001-ancs-feasibility-spike.md`

- Confirmed ANCS feasibility on the target Intel/BlueZ environment with fresh iPhone pairing and Freedesktop notification delivery.
- Demonstrated existing-bond restart and automatic disconnect/reconnect recovery without generic Bluetooth connection attempts or notification-payload persistence.
