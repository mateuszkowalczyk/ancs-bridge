# Iteration 006 — Runtime reliability acceptance

**Status:** Completed

## Sources

- `docs/prd/01-daemon-core.md`
- `docs/prd/04-validation-release.md`
- `docs/specs/ancs-session-forwarding.md`
- `docs/specs/runtime-machine-api.md`
- `docs/specs/systemd-user-service.md`
- `docs/tasks/archived/iteration-002-production-core.md`
- `docs/tasks/archived/iteration-005-systemd-user-service.md`

## Dependencies

- Iteration 005 provides an installed, enabled, service-managed bridge and an
  ANCS-authorized iPhone bond.
- Live acceptance requires the physical iPhone and a second device capable of
  sending representative notifications. AirPods are required only for the
  final Bluetooth and audio-invariant check.
- BlueZ restart, adapter power cycling, and suspend are operator-gated actions.
  Implementation must announce each disruptive
  step and wait for confirmation where the action cannot be safely automated.
- Evidence must remain metadata-only. Notification titles, subtitles, message
  bodies, and unique privacy-canary text must never be written to repository
  files or retained in diagnostics.

## Tasks

### Acceptance harness and regression coverage

- [x] Extend the opt-in hardware acceptance harness with bounded per-step and overall deadlines, explicit operator prompts, and metadata-only pass/fail records for each disruption scenario.
- [x] Add reusable baseline and post-scenario snapshots for service state, PID and restart count, machine status, configuration hash, Bluetooth bond set, adapter state, audio-suppression intent, WirePlumber health, and daemon RSS/file-descriptor counts.
- [x] Add common recovery assertions that require the bridge to return to `ready`, forward one post-recovery canary exactly once, avoid generic `Device1.Connect()` calls, and preserve the recorded configuration, bonds, and audio intent.
- [x] Map every live disruption in this iteration to deterministic fake BlueZ/clock or process-lifecycle coverage, and add regression tests for any recovery transition not already exercised automatically.

### Notification behavior and privacy

- [x] Document and execute repeatable hardware procedures for representative notifications while the iPhone is locked and unlocked and with previews set to Always, When Unlocked, and Never.
- [x] Validate live Added, Modified, and Removed events for one notification UID, confirming desktop create, replace, and close behavior without unintended duplicate notifications.
- [x] Inject a unique privacy canary and verify it is absent from the journal, configuration, runtime status, captured status/doctor/setup diagnostics, runtime files, and installed artifacts while retaining only metadata-only delivery evidence.

### Disruption and recovery matrix

- [x] Revalidate a deliberate daemon service restart with the common recovery assertions and a new process PID.
- [x] Restart BlueZ and verify bounded automatic recovery with the common recovery assertions.
- [x] Power the Bluetooth adapter off and on and verify bounded automatic recovery without the daemon changing the operator-selected adapter power state.
- [x] Turn iPhone Bluetooth off and on and verify passive bonded-device recovery without reopening iPhone Settings after the toggle.
- [x] Complete a suspend/lid cycle and verify post-resume reconciliation and forwarding without restarting the service manually.

### Endurance, invariants, and release readiness

- [x] Complete twenty harness-observed iPhone Shortcut Bluetooth off/on disconnect/passive-reconnect cycles without generic `Device1.Connect()`, requiring `ready` after every cycle, pre-run and post-run notification canaries without duplicates, no dead session, no unreleased per-session file descriptors, and no sustained monotonic RSS growth after warm-up.
- [x] Confirm the configured iPhone bond, unrelated Bluetooth bonds, audio-suppression intent, adapter state, and AirPods playback and microphone operation remain correct after the selected matrix.
- [x] Update operator documentation with the acceptance command, required hardware, disruptive-step expectations, timeout behavior, metadata-only evidence interpretation, and safe resume procedure after suspend or reboot.
- [x] Run formatting, Clippy with warnings denied, all non-hardware tests, the Linux dependency audit, locked release build, service-unit verification, staged-install inspection, and the selected live hardware matrix; finish with the service enabled, running, and `ready`.

## Deferred work

- immutable release tagging, version 0.1.0 release preparation, AUR `PKGBUILD`,
  `.SRCINFO`, `namcap`, clean package build/install/upgrade/removal validation,
  publication, and downstream installation documentation
- physical range-loss/return validation; the 20-cycle iPhone Bluetooth
  endurance run covers the same passive disconnect/reconnect path for this
  iteration, but does not reproduce radio range loss
- real reboot/login validation of automatic user-service startup; enablement,
  ordinary startup, and deliberate restart are covered in this iteration

## Implementation notes

- The 20-cycle iPhone Shortcut endurance run passed with passive reconnect on
  every cycle, one post-run notification delivery, a maximum of 11 file
  descriptors, and RSS changing from 8620 KiB to 8764 KiB.
- Live iOS 26.6 traffic used reserved event flag bit `0x20`. The ANCS codec now
  preserves reserved bits while interpreting known flags, preventing otherwise
  valid events such as the observed `0x35` mask from being discarded.
- The notification-preview matrix passed for TickTick and Apple Reminders with
  the iPhone locked and unlocked under Always, When Unlocked, and Never preview
  settings; every case produced exactly one desktop notification.
- One synced Apple Reminder exercised live Added and Modified delivery without
  a duplicate; clearing it produced one metadata-only Freedesktop
  `CloseNotification` call for Removed. The original desktop banner had already
  timed out before Modified, so same-UID replace selection is additionally
  covered by the deterministic session regression test.
- The privacy canary was delivered, then found zero times across 20 persistence
  surfaces totaling 7,790,137 bytes.
- BlueZ restart was revalidated after the registration-recovery fix with
  passive return to `ready`, preserved configuration/bonds/audio intent, and
  exactly one post-recovery notification.
- The first suspend run exposed a stale controller advertisement: BlueZ still
  reported one active instance, but the iPhone did not reconnect within five
  minutes. The supervisor now recreates its BlueZ session and registrations
  once after an active phone disconnect. A repeat suspend returned the same
  daemon PID to `ready` in about 35 seconds and forwarded one canary exactly
  once without generic connection calls or iPhone Settings interaction.
- Live iOS 26.6 also used reserved category ID `12`; the codec now retains
  reserved category values instead of discarding otherwise valid events.
- Final invariants passed with three bonds, adapter powered, exact-device phone
  audio suppression active, WirePlumber healthy, the iPhone absent from
  PipeWire, ANCS `ready`, and AirPods playback and microphone both working.
- Final validation passed formatting, warnings-denied Clippy, 78 non-hardware
  tests, the Linux dependency audit with the two documented Windows-only
  `quick-xml` advisory exceptions, locked release build, user-unit verification,
  staged artifact inspection, installed-artifact comparison, and the selected
  live matrix. The service finishes enabled, active, and `ready`.
