# ANCS session and notification forwarding

## Purpose

Define how the daemon discovers an authorized ANCS service, retrieves bounded
notification data, forwards notification lifecycle events to the Freedesktop
notification service, and recovers across iPhone disconnects.

## Discovery and authorization

The daemon recognizes the ANCS service and characteristics by UUID:

- ANCS service: `7905f431-b5ce-4e99-a40f-4b1e122d00d0`
- Notification Source: `9fbf120d-6301-42d9-8c58-25e699a21dbd`
- Data Source: `22eac6e9-24d6-4bb5-be44-b36ace7c7bfb`
- Control Point: `69d1d8f3-45e1-49a8-9821-9bbdfdaad9d9`

`ServicesResolved=true` does not prove that ANCS is already published or
authorized. While the configured iPhone remains connected, the daemon
reconciles service state and treats an absent, incomplete, or temporarily
unauthorized ANCS service as recoverable. It publishes
`waiting-for-authorization` when appropriate and retries without requiring a
new pairing or generic connection attempt.

The daemon does not enter `ready` until all three characteristics are present
and both subscriptions are active.

## Subscription and Control Point ordering

For every session, the daemon subscribes to Data Source before Notification
Source so it can receive attribute responses as soon as notification events
begin.

Control Point commands use an ATT write-with-response request. With `bluer`, the
operation must be selected explicitly as `WriteOp::Request`; its default
write-without-response command is not valid for ANCS Control Point requests.

Exactly one Control Point request may be outstanding. Each request has a
five-second timeout, and later work waits in a bounded queue. Timeout, an
ANCS-specific error, disappearing characteristics, or authorization failure is
recoverable and cannot terminate the daemon supervisor.

## Event and attribute processing

Notification Source events are exactly eight bytes. The daemon validates event
ID, category fields, and little-endian notification UID before acting on the
event. It preserves the complete event-flags bitmask, interprets the flags it
knows, and tolerates reserved flag bits for compatibility with newer iOS
versions. It likewise preserves reserved category IDs without assigning them
new semantics. Unknown event IDs and malformed values are rejected without
panicking.

For Added and relevant Modified events, the daemon retrieves:

- app identifier
- app display name
- title, limited to 256 bytes
- message, limited to 2048 bytes

Data Source responses may span arbitrary GATT notification boundaries. The
daemon incrementally reassembles complete attribute tuples, validates command
ID, UID, attribute ordering, lengths, and UTF-8, and enforces a 64 KiB hard cap
for buffered response data.

At most 100 notification UIDs may be pending. Modified events for the same UID
are coalesced, Removed cancels pending work, and `PreExisting` notifications are
skipped to avoid a notification flood when a session begins. App display names
are cached only in memory for the active ANCS session; after lookup failure or
timeout, the bundle identifier is used as the fallback display name.

## Desktop notification lifecycle

Each ANCS UID maps to one Freedesktop notification handle for the active
session:

- Added creates a notification.
- Modified replaces the mapped notification.
- Removed closes it.

V1 exposes no actions and never sends notification-action commands to the
iPhone. Delivery failure logs metadata only and does not terminate the ANCS
session.

Notification app, title, message, and other payload content remain in memory
only long enough to perform delivery. Payload content never appears in
configuration, runtime status, diagnostic JSON, or journal messages.

## Session end and reconnection

On disconnect or ANCS disappearance, the daemon cancels subscriptions and
pending requests, closes or clears mapped desktop notification handles, and
discards all session-scoped UIDs and cached app names.

The daemon retains or recreates its connectable, non-discoverable advertisement
and waits for the bonded iPhone to reconnect. Routine recovery never loops on
generic `Device1.Connect()`. After an incoming reconnection, the daemon waits
for ANCS publication/authorization, subscribes Data Source before Notification
Source, and begins a new session with no stale identifiers.

## Acceptance criteria

- Delayed ANCS publication after pairing or reconnect reaches `ready` without
  re-pairing.
- Data Source is subscribed before Notification Source on every session.
- Control Point attribute retrieval uses write-with-response and correctly
  handles fragmented, malformed, invalid-UTF-8, timed-out, and oversized data.
- Added, Modified, and Removed events produce the corresponding Freedesktop
  lifecycle without notification-payload persistence.
- Restart and iPhone disconnect/reconnect restore forwarding without generic
  connection attempts or repeated device selection.
