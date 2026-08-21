# Iteration 007 — Field validation and code audit

**Status:** Completed

## Sources

- `docs/prd/01-daemon-core.md`
- `docs/prd/02-machine-api.md`
- `docs/prd/03-packaging-security.md`
- `docs/prd/04-validation-release.md`
- `docs/specs/ancs-session-forwarding.md`
- `docs/specs/bluetooth-accessory-pairing.md`
- `docs/specs/phone-audio-suppression.md`
- `docs/specs/runtime-machine-api.md`
- `docs/specs/setup-and-device-lifecycle.md`
- `docs/specs/systemd-user-service.md`
- `docs/tasks/archived/iteration-006-runtime-reliability-acceptance.md`

## Dependencies and guardrails

- Live validation requires the installed, enabled, and configured service, the
  physical iPhone, enough distance to force a real radio range loss, one normal
  computer reboot/login, and AirPods for final audio invariants.
- The security review must be performed by a read-only subagent started with
  separate context (`fork_turns="none"`). The primary agent owns all edits and
  verifies every finding before acting on it.
- Refactors must preserve the current PRDs, specs, machine API v1 fixtures,
  privacy boundary, and external CLI behavior. A candidate that changes
  expected behavior requires discussion and explicit spec-change approval.
- Implement a refactor only when it has a clear net simplification and is
  protected by existing or newly added automated tests, or by one short,
  repeatable manual check. Defer uncertain or high-risk candidates instead.

## Tasks

### Independent security and architecture review

- [x] Start a fresh-context, read-only security-review subagent and have it inspect production code, CLI and JSONL authorization boundaries, BlueZ/D-Bus interactions, protocol parsing and resource bounds, filesystem permissions and path handling, subprocess execution, service lifecycle, privacy/logging, dependency exposure, and packaging helpers.
- [x] Record each security finding with severity, concrete evidence, exploitability or failure impact, and a recommended verification; explicitly record when a reviewed area has no confirmed finding.
- [x] Independently map module responsibilities and cross-module lifecycle paths, then identify duplication, unnecessary state, overly broad abstractions, and control flow that can be simplified without changing specified behavior.
- [x] Triage the combined findings into confirmed fixes, safe simplification candidates, false positives, and deferred ideas; add non-blocking deferred work to `docs/inbox.md` and explain why each implemented candidate clears the iteration's safety threshold.

### Safe remediation and simplification

- [x] Add or strengthen focused regression tests before changing any confirmed security-sensitive or insufficiently covered path.
- [x] Resolve every confirmed release-blocking, high-severity, or medium-severity security finding; if none exist, record that outcome and the evidence supporting it.
- [x] Implement only the selected low-risk architectural simplifications, keeping behavior and machine API fixtures unchanged; it is acceptable to make no refactor when no candidate has a clear safety and maintenance benefit.
- [x] Have the independent security reviewer re-check resolved findings and any security-sensitive refactor, and record whether each reviewed concern is closed or remains deferred.
- [x] Replace incomplete phone-audio suppression with a transactional user-level output-only Bluetooth role policy, preserving ANCS plus AirPods playback/microphone and retaining exact ownership, conflict, rollback, and teardown guarantees.

### Deferred live field validation

- [x] Exercise physical iPhone range loss and return with the acceptance harness, requiring passive recovery to `ready`, preserved configuration/bonds/audio intent, no generic `Device1.Connect()`, no iPhone Bluetooth Settings interaction, and exactly one post-recovery desktop notification.
- [x] Reboot the computer and log in normally, then verify the enabled user service starts without manual activation, reaches `ready`, preserves configuration/bonds/audio suppression, and forwards exactly one notification.
- [x] Finish the live checks with the configured iPhone having no active PipeWire audio profile/nodes, Omarchy absent from the iPhone audio-output picker, WirePlumber healthy, the service enabled/running/`ready`, and AirPods playback and microphone still working.

### Final verification

- [x] Run formatting, Clippy with warnings denied, all non-hardware tests, the Linux dependency audit, locked release build, service-unit verification, and `git diff --check`.
- [x] Run one short manual smoke check for every changed runtime path not fully exercised by the range-loss or reboot scenarios, and confirm diagnostics remain metadata-only with no notification payload persistence.

## Deferred work

- immutable `v0.1.0` release creation, making the upstream release source
  publicly downloadable, AUR `PKGBUILD` and `.SRCINFO`, `namcap`, clean-chroot
  package build/install/upgrade/removal validation, and AUR publication

## Implementation notes

### Security review and remediation

- A fresh-context read-only reviewer found no critical or high-severity issue.
  It confirmed three medium availability findings and two low hardening
  findings; all five received focused regression coverage before remediation.
- Medium: setup accepted unbounded JSONL lines and eagerly queued unbounded
  stdin in `src/setup/mod.rs`. A same-user frontend controlling stdin could
  exhaust memory while temporary BlueZ state was active and bypass normal
  cleanup through OOM. Setup now caps command lines at 4 KiB, bounds the reader
  channel to one line, rejects terminated and unterminated overflow as
  `invalid-protocol`, and preserves cleanup/commit ordering.
- Medium: `SessionEngine` capped pending UIDs but retained delivered desktop
  handles, app-name cache entries, and failed-request cancellation markers for
  the whole session. A configured malicious ANCS peer could grow them until
  daemon or notification-service degradation. Active handles and cached apps
  now use deterministic 100-entry FIFO bounds, evicted handles are closed, and
  cancellation markers are removed on every terminal request outcome. The user
  explicitly approved documenting the observable overflow behavior: after 100
  active mappings, the oldest notification is closed and forgotten, so a later
  Modified event for that evicted UID creates a new desktop notification.
- Medium: `Supervisor::handle_one_packet` drained pending ANCS work without a
  bound, while newly received events could replenish the queue. A configured
  peer could therefore starve BlueZ lifecycle reconciliation. The supervisor
  now processes one pending notification unit per outer loop and reconciles
  between units; the fake-transport ordering regression proves reconciliation
  occurs before a second notification's control writes.
- Low: ANCS responses could exceed the requested 256-byte title and 2,048-byte
  message limits while remaining under the 64 KiB decoder cap. The codec now
  rejects each attribute above its advertised maximum, including `+1` vectors.
- Low: `packaging/stage-install.sh` rejected only the literal `/`, so aliases
  such as `/./`, `/tmp/..`, or a symlink to root bypassed its safety check. The
  helper now canonicalizes `DESTDIR` before rejecting root, with safe tests for
  lexical and symlink aliases.
- The reviewer confirmed no additional finding in CLI authorization and
  request-ID validation, canonical BlueZ identity selection, encrypted HID
  shape, D-Bus trust gates, checked parser arithmetic, exact-device teardown,
  atomic owner-only persistence, normal lifecycle rollback, metadata-only
  logs/status, the unprivileged user unit, or staged artifact scope.
- Its read-only closure pass marked all three medium and both low findings
  closed, found no new security issue, and confirmed the status-writer
  simplification introduces no privacy or identity regression.
- The two `quick-xml 0.31.0` RustSec advisories remain target-inactive through
  the Windows-only `tauri-winrt-notification` dependency; the locked Linux tree
  contains no `quick-xml`, so the existing narrow audit ignores remain valid.
  The `bluer 0.17.3` future-incompatibility warning is unchanged.
- PATH-resolved `systemctl`/`bluetoothctl`, same-user status PID spoofing,
  owner-controlled parent symlinks, and possible extra systemd sandboxing were
  triaged as non-exploitable in the current same-user threat model or
  defense-in-depth, not release findings. Broader sandbox compatibility work is
  deferred in `docs/inbox.md`.

### Architecture review and simplification

- The lifecycle map remains intentionally layered: `main` composes CLI and
  environment stores; setup protocol owns authorization/state transitions;
  the production setup backend owns temporary BlueZ state and commit rollback;
  transport owns BlueZ RAII objects; supervisor owns reconciliation; session
  owns bounded payload-bearing state; notification owns Freedesktop handles;
  status persists metadata only; teardown reverses configured state in order.
- The selected simplification removes the redundant immutable
  `StatusIdentity` copy from `PersistentStatusWriter`. Identity is now copied
  into its private current status exactly once at construction instead of
  being stored twice and recopied on every publication. Existing timestamp and
  persistence tests were strengthened to assert stable adapter, address, and
  device name before the production change; machine API fixtures are unchanged.
- Repeated environment-path helpers and supervisor flag assignments were left
  explicit because extracting them would add abstraction without a clear net
  reduction. The large setup backend was not split because its transaction and
  rollback order is easier to audit in one place. Typed setup audio errors were
  identified as a useful future cleanup but deferred in `docs/inbox.md` because
  the current string-classification branches are covered and the broader
  hardware-sensitive refactor does not clear this iteration's safety threshold.
- The reviewer confirmed that reconciliation can no longer be starved
  indefinitely. Whether the five-second interval must be a strict wall-clock
  deadline during the two request windows needed for a notification plus an
  uncached app lookup is a non-security product clarification, deferred in
  `docs/inbox.md`; implementing cancellation-safe interleaving without that
  decision would not clear this iteration's safety threshold.

### Automated verification

- The pre-change baseline passed all 78 non-hardware tests. After remediation,
  formatting, warnings-denied Clippy, 92 non-hardware tests, the Linux-relevant
  dependency audit, locked release build, user-unit verification, and
  `git diff --check` pass. Machine API v1 fixtures include the approved
  additive `adapterAddress` status field while retaining version 1.

### Field-discovered controller identity migration

- The first live prerequisite check found the configured controller recorded as
  `hci0` while BlueZ exposed the same physical controller as `hci1`; the exact
  configured iPhone bond remained paired and trusted on that controller. This
  confirmed that the kernel-assigned adapter name is not stable enough to be
  the runtime identity across re-enumeration or reboot.
- With explicit user approval, the version 1 configuration now optionally
  persists the controller's public Bluetooth address as its stable key and
  keeps `adapter` as the last-known display name and legacy fallback. Runtime,
  diagnostics, teardown, and the acceptance harness resolve the current
  `hciN` name by exact controller address and fail rather than select by
  position or accept an ambiguous match.
- Legacy setup migration is deliberately narrow: when the recorded adapter is
  absent, exactly one controller must contain the exact configured paired
  iPhone. Setup then advertises the existing encrypted accessory, requires the
  ordinary caller confirmation, verifies ANCS, and atomically commits the
  stable controller address and current adapter name. Zero or multiple exact
  matches leave configuration unchanged.
- The focused independent re-review found no new critical, high, or medium
  issue and confirmed all earlier security findings remain closed. Its two low
  correctness findings were addressed: status now includes the optional
  metadata-only controller address in freshness matching, and the hardware
  harness resolves only direct adapter objects, rejects ambiguous identity
  matches, and power-cycles the exact configured adapter through D-Bus.

### Field-discovered phone audio routing

- Live inspection showed the exact-device WirePlumber rule was loaded and set
  `device.disabled=true`, yet BlueZ still held an active iPhone A2DP transport
  and iOS selected Omarchy as a speaker. The rule suppressed local nodes only;
  it could not prevent WirePlumber's globally registered sink roles from being
  advertised before the peer identity was known.
- The user approved a user-level output-only role policy for all Bluetooth
  peers in the logged-in WirePlumber session. It retains `a2dp_source`,
  `bap_source`, and `hfp_ag` for AirPods-class playback and microphones, while
  omitting roles that let phones use Omarchy as a speaker/headset. No `/etc`,
  system-wide BlueZ configuration, or root write is introduced.
- The exact-device and output-only files are now reconciled from previous to
  desired setup intent as one transaction. Tests cover enable, disable,
  repeated enable, phone-identity replacement, missing-rule repair,
  configuration-write rollback, conflict preflight, teardown rollback reload,
  and double restart failure. Configuration remains the last commit and failed
  rollback never hides cleanup failure.
- Final acceptance now requires enabled suppression intent, exact canonical
  bytes for both rules, an off/disabled configured-phone card when present, no
  configured-phone audio nodes, absence of classic phone-facing controller
  roles, absence from the iPhone output picker, and working AirPods playback
  and microphone.
- The independent reviewer closed the audio-policy follow-up after verifying
  the transaction and rollback paths, exact `wpctl` object-ID parsing, privacy
  boundary, and the final PRD, spec, and operator-documentation consistency;
  no security, lifecycle, or documentation blocker remains in this scope.
- Live migration resolved the legacy `hci0` configuration to controller address
  `4C:A9:54:EC:4B:49` at current name `hci1`, reused the approved existing
  iPhone bond, committed both canonical user-level audio rules, and returned the
  enabled daemon to a fresh `ready` subscription after an iPhone Bluetooth
  cycle. Omarchy was absent from the iPhone output picker and the final harness
  confirmed no phone audio nodes or forbidden controller roles while AirPods
  playback and microphone both worked.
- One TickTick canary was forwarded to the desktop, then the privacy harness
  scanned configuration, machine diagnostics, runtime files, service status and
  journal, installed artifacts, and API fixtures (20 surfaces and 7,989,119
  bytes) without finding the notification text.
- The physical range-loss stage observed a genuine disconnect and then passive
  recovery to `ready` without opening iPhone Bluetooth Settings. The daemon PID
  remained unchanged, no generic `Device1.Connect()` was used, configuration,
  bonds, adapter power, audio intent, and WirePlumber health were preserved, and
  exactly one post-recovery notification reached the desktop.
- After a normal reboot and login, the enabled user service started without
  manual activation and reached a fresh `ready` state under its new process.
  Configuration, bonds, adapter power, audio suppression, and WirePlumber health
  remained intact, exactly one post-reboot notification reached the desktop,
  and the harness emitted metadata only.
