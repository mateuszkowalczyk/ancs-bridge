# Bluetooth accessory and pairing

## Purpose

Define the Bluetooth LE accessory shape and pairing behavior that allows one
configured iPhone to authorize ANCS without patched BlueZ or iOS. This spec
covers setup-time pairing and the local accessory objects retained by the
production daemon; the setup JSONL wire format remains defined by
`docs/prd/02-machine-api.md`.

## Local HID accessory

The daemon registers a minimal HID-over-GATT keyboard service containing:

- HID Information
- Report Map
- HID Control Point
- Protocol Mode
- one input Report
- a Report Reference descriptor for that input report

The report map presents a keyboard-shaped input report, but the daemon never
sends input reports, key codes, or keyboard events.

Reads of HID Information, Report Map, Protocol Mode, Report, and Report
Reference require an encrypted connection. Writes to HID Control Point and
Protocol Mode also require encryption. The encryption requirements must cause
iOS to establish a bond before it can use the accessory service.

The daemon registers the GATT application before advertising and keeps the
application handle alive for the process lifetime. During setup it advertises
as discoverable/connectable with the HID service UUID and ANCS
service-solicitation UUID. During normal runtime it advertises as connectable
and non-discoverable.

The daemon never forces adapter power. If the selected adapter is powered off,
setup reports that state to the caller rather than changing it.

## Pairing authorization

Pairing is initiated by the user from iPhone Bluetooth settings. Before opening
the pairing window, setup must:

1. capture the adapter's pairable and discoverable values
2. register the encrypted HID GATT application
3. register the setup advertisement
4. register a temporary `DisplayYesNo` or `KeyboardDisplay` BlueZ agent

Only one confirmation request may be active. The caller receives the incoming
device name, identity address, and six-digit passkey when BlueZ supplies one.
Setup accepts the request only after the caller confirms the matching opaque
request ID. A device name or advertised local name is display information, not
identity; the bonded BlueZ identity address is the persisted device key.

No other device is accepted while confirmation is pending. After approval,
setup verifies that the same device is paired and bonded, marks it trusted, and
records it only after all remaining setup operations succeed.

An existing bond may be reused only when diagnostics confirm ANCS readiness.
Otherwise, forgetting and re-pairing the device requires explicit caller
authorization.

## Cleanup and failure behavior

Success, rejection, cancellation, confirmation timeout, stdin closure, and
unexpected returned errors all restore the captured adapter values and
unregister temporary agent, advertisement, and GATT objects. Partial
configuration is never persisted.

BlueZ-owned temporary objects must also disappear when the process loses its
D-Bus connection. Setup documentation may describe manual adapter restoration
after failures such as `SIGKILL` or power loss that cannot execute in-process
cleanup.

## Acceptance criteria

- A fresh pairing initiated from iPhone settings succeeds on a supported
  adapter without patched BlueZ or iOS.
- The iPhone bonds only after encrypted HID access and only after the caller
  confirms the exact incoming request.
- No keyboard input is ever emitted.
- Cancellation and failure leave no partial configuration or temporary BlueZ
  registrations and restore adapter pairable/discoverable state.
- The production daemon retains the local GATT application and appropriate
  advertisement for its full lifetime.

