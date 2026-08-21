# Phone audio suppression and output-only Bluetooth roles

## Purpose

Prevent the configured iPhone from becoming an active PipeWire audio endpoint
and prevent the desktop from advertising itself to phones as a Bluetooth
speaker or headset. Preserve ANCS/GATT behavior plus playback and microphone
support for output devices such as AirPods.

## Rule identity and content

For validated address `AA:BB:CC:DD:EE:FF`, the owned rule path is:

```text
$XDG_CONFIG_HOME/wireplumber/wireplumber.conf.d/90-ancs-bridge-AA_BB_CC_DD_EE_FF.conf
```

`XDG_CONFIG_HOME` falls back to `~/.config`. Colons are replaced with
underscores only after parsing and canonicalizing the Bluetooth identity
address. No unvalidated value participates in path or rule generation.

The canonical exact-device rule is:

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

It matches no wildcard, node, or other address.

The bridge also owns this user-level role-policy path:

```text
$XDG_CONFIG_HOME/wireplumber/wireplumber.conf.d/91-ancs-bridge-bluetooth-output-only.conf
```

Its canonical content is:

```ini
monitor.bluez.properties = {
  bluez5.roles = [ a2dp_source bap_source hfp_ag ]
}
```

Bluetooth audio roles are registered before a peer identity is known, so this
policy necessarily applies to every Bluetooth peer handled by the logged-in
user's WirePlumber instance. `a2dp_source` and `bap_source` retain high-quality
classic and LE playback to headphones/speakers; `hfp_ag` retains headset
microphone/call audio. Local sink and hands-free roles are omitted so a phone
cannot select the desktop as an audio destination. This writes no system-wide
BlueZ or `/etc` configuration and requires no root access.

## Application and removal

Creation uses atomic replacement and mode `0600`. Missing owned directories
are created privately; existing parent-directory permissions are not broadened
or otherwise rewritten.

The exact-device and output-only rules are preflighted and applied or removed
as one transaction. Applying an already identical rule set and removing an
already absent rule set are successful no-ops. If either owned path contains
different content, setup or teardown reports `audio-rule-conflict` before
changing either path rather than overwriting or deleting unrecognized user
data.

After a rule is created or removed, run:

```text
systemctl --user restart wireplumber.service
```

At most one restart occurs for each successful multi-rule apply or removal.
No restart occurs for a true no-op. A failed restart is an operation failure.
Setup rollback removes newly created rules and restores rules removed or
replaced during intent/identity reconciliation, then reloads the restored
policy. Teardown likewise restores removed canonical rules and reloads the
rollback. Configuration is retained whenever cleanup must be retried.

WirePlumber is required for every configured bridge. An unavailable
WirePlumber service, an unavailable user service manager, a conflicting rule,
or a failed restart prevents setup completion and configuration commit.

## Ownership and rollback

The bridge always owns the two canonical paths and their exact canonical
content. It never scans wildcard paths for deletion.

When a later setup step fails, rollback reverses the complete rule-set change:
it removes only rules newly created by that transaction, restores any previous
rules removed or replaced during disable or identity migration, and restarts
WirePlumber. A rule that existed identically before setup is not removed by
rollback. Rollback failures are surfaced as cleanup failures and never hidden
by the original error.

## Acceptance criteria

- Path generation rejects malformed addresses and produces both canonical
  paths and exact content.
- Multi-rule application, repeated application, removal, and repeated removal
  are safe and deterministic; conflicting content is preserved and changes
  trigger at most one WirePlumber restart.
- With suppression active, the configured iPhone has no active PipeWire audio
  profile or nodes, Omarchy is not offered as an iPhone audio destination,
  ANCS still reaches `ready`, and AirPods playback and microphone remain
  functional.
- Teardown and setup rollback remove only bridge-created content and leave
  WirePlumber healthy.
