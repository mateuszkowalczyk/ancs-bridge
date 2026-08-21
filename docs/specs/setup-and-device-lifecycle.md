# Setup, diagnostics, and device lifecycle

## Purpose

Define the machine-readable diagnostics, interactive setup transaction, and
teardown behavior for one iPhone. Bluetooth accessory details remain governed
by `bluetooth-accessory-pairing.md`, audio-rule details by
`phone-audio-suppression.md`, and runtime forwarding by
`ancs-session-forwarding.md`.

## Adapter selection

Diagnostics and setup resolve a configured controller by its stable public
Bluetooth address, tolerating a changed BlueZ `hciN` name. Otherwise they use
the only adapter exposed by BlueZ. They never guess when BlueZ exposes zero or
multiple adapters, and they never power an adapter.

For a legacy version 1 configuration without a controller address, setup first
tries the recorded adapter name. If that name disappeared, it may select a
single adapter only when that adapter contains the exact configured paired
iPhone identity. It registers the ordinary runtime advertisement, asks the
caller to confirm reuse of that existing bond, verifies ANCS, and writes the
stable controller address plus current `hciN` name during the normal commit.
Zero or multiple exact-bond matches fail without changing configuration.

Setup fails before opening a pairing window unless the selected adapter is
powered and supports the required central and peripheral roles plus LE
advertising. All adapter names, controller addresses, and device identity
addresses are validated before use in D-Bus paths or persisted configuration.

## `doctor`

`ancs-bridge doctor` writes exactly one object with this shape:

```json
{
  "apiVersion": 1,
  "ok": true,
  "checks": [
    {"id": "bluez-version", "status": "pass", "code": null}
  ]
}
```

Every check contains only `id`, `status`, and `code`. Valid statuses are
`pass`, `warn`, and `fail`; `ok` is false if any check fails. The stable check
IDs are:

- `bluez-version`
- `adapter-power`
- `adapter-roles`
- `le-advertising`
- `existing-pairing`
- `wireplumber`
- `ancs-readiness`

BlueZ 5.87 is the validated baseline for the initial hardware alpha. Missing or
unreadable BlueZ is a failure; another parseable version is reported with a
stable unvalidated-version warning rather than rejected solely by its number.
No unambiguous adapter, a powered-off adapter, missing required roles, or
unavailable LE advertising is a failure. A missing pairing is a warning when
unconfigured and a failure when configuration names that missing bond.
WirePlumber absence is a warning before setup and a failure for a configured
bridge. A disconnected phone or ANCS that is not currently ready is a warning
because setup or normal reconnection may resolve it.

Doctor emits its complete JSON result even when checks fail and exits nonzero
when `ok` is false. A failure that prevents the result itself from being
constructed leaves stdout empty, writes a concise diagnostic to stderr, and
exits nonzero. No notification payload or localized prose appears in JSON.

## Setup command and protocol

The command is:

```text
ancs-bridge setup [--repair]
```

Setup reads one JSON command per stdin line and emits one JSON event per stdout
line. JSON Lines are flushed immediately. Diagnostics use stderr only.

Stable state events are:

```json
{"v":1,"event":"state","state":"checking-environment"}
{"v":1,"event":"state","state":"waiting-for-iphone"}
{"v":1,"event":"state","state":"verifying-ancs"}
{"v":1,"event":"state","state":"applying-configuration"}
```

Confirmation uses the existing v1 event with one additive `kind` field and a
nullable passkey:

```json
{"v":1,"event":"confirmation-request","kind":"pairing","requestId":"opaque-id","deviceName":"iPhone","address":"AA:BB:CC:DD:EE:FF","passkey":"123456"}
{"v":1,"event":"confirmation-request","kind":"existing-bond","requestId":"opaque-id","deviceName":"iPhone","address":"AA:BB:CC:DD:EE:FF","passkey":null}
```

Valid caller commands remain:

```json
{"v":1,"command":"confirm","requestId":"opaque-id","accept":true}
{"v":1,"command":"cancel"}
```

Only one confirmation may be active. A confirmation ID is opaque and must
match exactly. Unknown versions, commands, fields with invalid types, commands
sent in the wrong state, and mismatched request IDs are fatal protocol errors
and cannot authorize a BlueZ operation.

Setup waits at most five minutes for an iPhone candidate, 30 seconds for caller
confirmation, and 60 seconds after pairing or reuse for ANCS readiness. These
timeouts use an injectable monotonic clock in automated tests.

## Existing bonds and explicit repair

A configured bond or a unique ANCS-ready existing bond may be reused only
after the caller accepts an `existing-bond` confirmation. Reuse verifies the
same identity address is paired, trusted, connected, services-resolved, and
publishes the complete ANCS service and characteristics. Verification may
temporarily subscribe Data Source before Notification Source, but it sends no
notification action or generic `Device1.Connect()` request.

If the configured bond is not ANCS-ready, setup without `--repair` emits
`repair-required` without removing the bond or changing configuration. With
`--repair`, setup may forget only that exact configured identity after all
environment checks pass, then waits for a fresh iPhone-initiated pairing.
`--repair` without a configured identity fails with `repair-target-unknown`;
setup never guesses which existing bond to remove.

Fresh setup records the set of existing bonds before becoming pairable and
accepts only a new candidate explicitly confirmed by the caller. While a
confirmation is pending, every other device is rejected.

## Pairing and commit transaction

Before opening the pairing window, setup captures pairable and discoverable
adapter values, registers the encrypted HID application, registers the
discoverable/connectable setup advertisement, and registers a temporary
`DisplayYesNo` or `KeyboardDisplay` agent.

After caller approval, setup verifies the same identity is bonded, marks it
trusted, waits for complete ANCS readiness, and then removes temporary ANCS
subscriptions. It restores adapter settings and unregisters the temporary
agent, advertisement, and GATT application before applying persistent changes.

Setup next reconciles the previously configured phone identity to the newly
confirmed identity while always applying both audio rules. This can create both
rules, repair a missing canonical rule, or replace only the exact-device rule
when the confirmed identity changes. The two rule paths are preflighted and
changed as one transaction with at most one WirePlumber restart. Configuration
is written atomically last. If configuration persistence fails, setup restores
the prior rule set and reloads WirePlumber before reporting failure.

Success is emitted only after every required step succeeds:

```json
{"v":1,"event":"complete","address":"AA:BB:CC:DD:EE:FF"}
```

Controlled failure emits one final error event and exits nonzero:

```json
{"v":1,"event":"error","code":"stable-error-code","recoverable":true}
```

Stable error codes cover environment/adapter failure, timeout, cancellation,
rejection, invalid protocol input, unsupported API version, stdin closure,
pairing/trust failure, repair authorization, ANCS readiness, audio-rule work,
configuration persistence, and cleanup failure. Their exact values and
recoverability flags are committed as machine API v1 fixtures.

Cancellation, rejection, timeout, stdin closure, SIGINT, SIGTERM, and returned
errors all attempt full temporary cleanup and adapter restoration. BlueZ-owned
objects also disappear with the D-Bus connection. Documentation must describe
manual adapter restoration after SIGKILL or power loss, which cannot execute
in-process cleanup.

## Teardown

`ancs-bridge teardown [--forget-device]` is idempotent and produces no stdout.
Success may be silent; human diagnostics and errors use stderr.

Teardown performs these operations in order:

1. stop and disable `ancs-bridge.service` when the user unit exists
2. remove the configured exact-device and output-only audio rules and reload
   WirePlumber when required
3. remove only the configured BlueZ device when `--forget-device` is present
4. remove configuration last

Independent cleanup is best-effort, but configuration is retained whenever a
required cleanup step fails so the command can be retried safely. An
absent service, already absent owned rules, already absent pairing, or absent
configuration is not an error. Without valid configuration teardown never
guesses a device address or scans and removes matching files.

## Acceptance criteria

- Doctor produces stable, payload-free checks and correctly distinguishes
  required failures from optional or transient warnings.
- Fresh pairing, caller rejection, timeout, cancellation, stdin closure,
  wrong-device requests, existing-bond reuse, and explicitly authorized repair
  are deterministic and leave no unintended configuration or temporary BlueZ
  state.
- Setup never powers the adapter, guesses among adapters/devices, accepts an
  unconfirmed identity, or uses generic connection attempts.
- `complete` is impossible before ANCS readiness, cleanup, audio suppression,
  and atomic configuration persistence all succeed.
- Teardown retaining the bond and teardown forgetting the bond both remove
  only bridge-owned state and can be rerun safely.
