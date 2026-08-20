# Exact-device phone audio suppression

## Purpose

Prevent only the configured iPhone from appearing as a PipeWire audio device
without changing Bluetooth roles, ANCS/GATT behavior, or unrelated devices
such as AirPods.

## Rule identity and content

For validated address `AA:BB:CC:DD:EE:FF`, the owned rule path is:

```text
$XDG_CONFIG_HOME/wireplumber/wireplumber.conf.d/90-ancs-bridge-AA_BB_CC_DD_EE_FF.conf
```

`XDG_CONFIG_HOME` falls back to `~/.config`. Colons are replaced with
underscores only after parsing and canonicalizing the Bluetooth identity
address. No unvalidated value participates in path or rule generation.

The canonical rule is:

```ini
monitor.bluez.rules = [
  {
    matches = [
      { device.name = "bluez_card.AA_BB_CC_DD_EE_FF" }
    ]
    actions = {
      update-props = {
        device.disabled = true
      }
    }
  }
]
```

It matches no wildcard, global Bluetooth role, node, or other address.

## Application and removal

Creation uses atomic replacement and mode `0600`. Missing owned directories
are created privately; existing parent-directory permissions are not broadened
or otherwise rewritten.

Applying an already identical rule and removing an already absent rule are
successful no-ops. If the owned path contains different content, setup or
teardown reports `audio-rule-conflict` rather than overwriting or deleting
unrecognized user data.

After a rule is created or removed, run:

```text
systemctl --user restart wireplumber.service
```

No restart occurs for a true no-op. A failed restart is an operation failure;
the setup transaction rolls back a newly created rule, while teardown retains
configuration so the cleanup can be retried.

WirePlumber is optional when suppression is not requested. When
`--disable-phone-audio` is selected, unavailable WirePlumber, an unavailable
user service manager, a conflicting rule, or a failed restart prevents setup
completion and configuration commit.

## Ownership and rollback

The bridge records suppression intent in configuration. It owns only the one
canonical path derived from that configured address and exact canonical
content. It never scans wildcard paths for deletion.

When setup created the rule but a later step fails, rollback removes that
exact new rule and restarts WirePlumber. A rule that existed identically before
setup is not removed by rollback. Rollback failures are surfaced as cleanup
failures and never hidden by the original error.

## Acceptance criteria

- Path generation rejects malformed addresses and produces the canonical
  exact-address filename and match.
- Application, repeated application, removal, and repeated removal are safe
  and deterministic; conflicting content is preserved.
- With suppression active, only the configured iPhone audio card is disabled,
  ANCS still reaches `ready`, and AirPods playback and microphone remain
  functional.
- Teardown and setup rollback remove only bridge-created content and leave
  WirePlumber healthy.
