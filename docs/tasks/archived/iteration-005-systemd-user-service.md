# Iteration 005 — systemd user service

**Status:** Completed

## Sources

- `docs/prd/01-daemon-core.md`
- `docs/prd/03-packaging-security.md`
- `docs/prd/04-validation-release.md`
- `docs/specs/runtime-machine-api.md`
- `docs/specs/setup-and-device-lifecycle.md`
- `docs/specs/systemd-user-service.md`
- `docs/tasks/archived/iteration-004-setup-device-lifecycle.md`

## Dependencies

- Iteration 004 provides validated setup, configuration persistence, doctor,
  status, teardown, and a hardware-authorized iPhone bond.
- Live service installation and lifecycle checks require explicit approval for
  system-path and user-service-manager changes during implementation.

## Tasks

### Unit and install artifacts

- [x] Add the production `ancs-bridge.service` unit with the required executable path, restart policy, runtime directory, umask, default-target install section, and baseline hardening.
- [x] Add automated unit-contract tests that reject missing or changed required directives, unexpected privilege requirements, and secret or payload-bearing environment configuration.
- [x] Add a reproducible staged source-install check that installs only the release binary, license, and user unit at their final paths with appropriate modes and no user-state mutation.
- [x] Validate the exact checked-in unit with `systemd-analyze verify`, including its installed executable path in an isolated staging root.

### Activation and lifecycle behavior

- [x] Preserve explicit activation policy: setup and installation do not enable or start the bridge service, while teardown retains its absent-unit and stop/disable guarantees.
- [x] Validate explicit `enable --now` after successful setup, confirming the installed service reaches `ready` and owns the expected runtime status.
- [x] Inject an unexpected daemon-process failure and verify systemd starts a new PID after the configured delay and forwarding returns to `ready` without device reselection or generic connection attempts.
- [x] Validate deliberate stop/start and disable/enable cycles, confirming configuration, the exact iPhone bond, audio-suppression intent, and unrelated Bluetooth bonds remain intact.
- [x] Forward a hardware notification through the service-managed daemon and record metadata-only evidence of successful delivery.
- [x] Inspect service status and journal evidence to verify that notification content and secret values are absent.

### Operator documentation and validation

- [x] Document release build, reversible manual installation to final system paths, daemon reload, explicit enable/start, service and machine-status inspection, logs, stop/disable, teardown, and manual removal.
- [x] Clearly distinguish the temporary source-install workflow from deferred AUR packaging and retain foreground daemon instructions.
- [x] Run formatting, Clippy with warnings denied, all automated tests, Linux dependency audit, locked release build, unit verification, staged-install inspection, and the live service hardware checks.

## Deferred work

- the remaining runtime reliability and hardware-acceptance matrix, including BlueZ restart, suspend/lid, range loss, reboot/login, representative notification lifecycle cases, privacy canaries across every persistent surface, and twenty reconnect cycles
- immutable release tagging, AUR `PKGBUILD`, `.SRCINFO`, `namcap`, clean package install/upgrade/removal validation, publication, and release 0.1.0

## Implementation notes

- Repository validation passes with 72 non-hardware tests, formatting, Clippy with warnings denied, the Linux dependency audit, a locked release build, staged artifact inspection, and isolated `systemd-analyze verify`.
- The manually installed binary, license, and user unit match repository hashes and use root-owned `0755`/`0644` modes. Installation left the service disabled and inactive until explicit activation.
- Live service evidence on 2026-08-20 confirms `ready`, owner-only runtime state, SIGKILL restart to a new PID after the configured delay, deliberate stop/start and disable/enable behavior, unchanged configuration/audio intent and Bluetooth bonds, and a final enabled/running service.
- A hardware notification advanced `lastNotificationAt` and appeared on the desktop. Its unique canary was absent from the service journal, configuration, runtime status, installed binary, unit, and license; the journal contained lifecycle metadata only.
