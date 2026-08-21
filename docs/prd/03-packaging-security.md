# PRD: Daemon Packaging and Security

## AUR package

- Package name: `ancs-bridge`.
- License: MIT.
- Build from an immutable GitHub release tag/tarball with pinned SHA-256 checksum.
- Commit and use `Cargo.lock`; build with `cargo build --release --locked`.
- Install only `/usr/bin/ancs-bridge`, the license, and the systemd user unit.
- Rust/Cargo are make dependencies.
- BlueZ/D-Bus are runtime dependencies.
- WirePlumber is a runtime dependency because every configured bridge applies
  the exact-device and user-level output-only Bluetooth role policy.
- Do not ship a prebuilt binary in the AUR source package.
- Do not use package install hooks to enable services, pair devices, modify user configuration, or restart WirePlumber.
- Generate and commit `.SRCINFO`; validate with `makepkg --cleanbuild` and `namcap` in a clean Arch environment.

## systemd user service

Install `/usr/lib/systemd/user/ancs-bridge.service` with:

```ini
ExecStart=/usr/bin/ancs-bridge daemon
Restart=on-failure
RestartSec=3
RuntimeDirectory=ancs-bridge
UMask=0077
```

- Enable under the user's default target only after setup succeeds; the package itself never enables it.
- Run as the logged-in user without root, sudo, or an Omarchy dependency.
- Permit required D-Bus access to BlueZ and the desktop notification service; no network access is needed.
- Add conservative hardening such as `NoNewPrivileges=true` and `PrivateTmp=true`, shipping only directives proven compatible with BlueZ and notification D-Bus access.
- Restart must preserve persistent configuration and recreate runtime status and BlueZ registrations.

## WirePlumber phone-audio policy

For every successful setup, generate atomically:

`~/.config/wireplumber/wireplumber.conf.d/90-ancs-bridge-AA_BB_CC_DD_EE_FF.conf`

Match only:

```ini
device.name = "bluez_card.AA_BB_CC_DD_EE_FF"
```

and apply:

```ini
device.disabled = true
```

Validate and normalize the address before path/rule generation. Application
and removal are idempotent. Restart the user's WirePlumber service after a
change. The exact-device rule never matches another device. The implementation
follows [WirePlumber Bluetooth rules](https://pipewire.pages.freedesktop.org/wireplumber/daemon/configuration/bluetooth.html)
and [`device.disabled`](https://pipewire.pages.freedesktop.org/pipewire/page_man_pipewire-props_7.html).

Also generate a user-level
`91-ancs-bridge-bluetooth-output-only.conf` policy that retains only
`a2dp_source`, `bap_source`, and `hfp_ag`. This affects all Bluetooth peers in
the logged-in user's WirePlumber session because roles are registered before a
peer identity is known, but it never modifies `/etc` or system-wide BlueZ
configuration. Apply, remove, and roll back both exact canonical files as one
transaction with at most one successful-operation reload.

## Privacy and security

- Notification titles, bodies, and app payloads exist only in memory long enough to deliver the Linux notification.
- Never write notification content to configuration, runtime status, diagnostic JSON, or journal logs.
- Configuration is mode `0600`; runtime files use `RuntimeDirectory` and `UMask=0077`.
- Setup accepts only the single device explicitly confirmed by the caller.
- Validate addresses, API input, lengths, command IDs, and all untrusted Bluetooth data.
- Bound ANCS buffers, queues, attribute lengths, and request timeouts.
- Keep machine-readable stdout separate from stderr diagnostics.
- Run without analytics, telemetry, or network access.
- Teardown is idempotent and removes only configuration and rules belonging to the configured bridge.

## Upgrade and removal

- Package upgrades preserve user configuration and service enablement.
- Package removal does not silently delete user configuration or pairing.
- `ancs-bridge teardown` removes configuration and both generated audio rules;
  `--forget-device` additionally removes the pairing.
- A frontend should perform teardown before optional package removal, but the CLI must remain usable without a frontend.

## Acceptance criteria

- Clean build, install, upgrade, and package removal succeed.
- Installed files are limited to the binary, license, and user unit.
- The package performs no automatic user-state mutation.
- The user service runs without root and restarts after failure.
- No notification content or secret appears in package logs, systemd journal, or machine status.
- The exact-device rule affects only the configured phone; the documented
  user-level output-only role policy affects all Bluetooth peers and preserves
  headphone playback/microphone roles. Both are fully reversible.
