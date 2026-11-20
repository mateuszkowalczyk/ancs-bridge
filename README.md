# ancs-bridge

`ancs-bridge` is for Linux desktop users who want iPhone notifications without
an iPhone companion app or a cloud relay. It uses Apple’s native Notification
Center Service (ANCS) over Bluetooth LE and forwards notifications locally to
the desktop.

It is a good choice when security and privacy matter: it works locally, sends
no data to a cloud service, and has no analytics or telemetry. It only reads
notifications and shows them on the desktop; it never sends actions back to
the phone. Notification text is not saved, and setup always asks you to
confirm the phone you are pairing.

## Install from source on Arch Linux

`ancs-bridge` is not available in the AUR because new AUR account registration
is currently disabled. Use an x86_64 Arch system with network access,
`base-devel`, and `git`. The package recipe declares the remaining build and
runtime dependencies.

```console
git clone https://github.com/mateuszkowalczyk/ancs-bridge.git
cd ancs-bridge
git checkout --detach refs/tags/v0.2.0 && cat PKGBUILD
```

The release's `PKGBUILD` pins and verifies the source archive. After reviewing
the recipe, build and install the package as a regular user:

```console
makepkg --syncdeps --install
```

Installation does not pair your phone or enable or start the service.

## Set up your iPhone

The daemon is designed primarily to integrate with a desktop UI or widget
that handles pairing prompts. If you set it up manually in a terminal,
you'll need to paste a JSON command to confirm the pairing as described below.

1. Before setup, remove any existing pairing between the iPhone and computer
   on both devices. On the iPhone, tap the information button next to your
   computer's Bluetooth name, then tap **Forget This Device**. On the computer,
   remove the iPhone from the list of paired Bluetooth devices.

   If either device was manually unpaired after a previous setup, run
   `ancs-bridge setup --repair` instead of the plain setup command below.

2. Start setup and leave the terminal open:

   ```console
   ancs-bridge setup
   ```

3. On your iPhone, open **Settings → Bluetooth**.

4. Under **Other Devices**, tap your computer. It may appear as **ANCS Bridge**
   or under its Bluetooth name. Start pairing from the iPhone, not the
   computer's Bluetooth settings.

5. When the terminal shows a `confirmation-request`, check that its phone and
   pairing code match your iPhone. Then paste this line into the same terminal:

   ```json
   { "v": 1, "command": "confirm" }
   ```

   Use `{"v":1,"command":"reject"}` to decline the displayed phone, or
   `{"v":1,"command":"cancel"}` to abort setup.

6. Wait until the terminal shows `"event":"complete"`.

7. Start the notification service and make it start automatically when you log
   in:

   ```console
   systemctl --user enable --now ancs-bridge.service
   ```

8. Wait a few seconds, then check that the service reports `"state":"ready"`:

   ```console
   ancs-bridge status
   ```

9. Create a reminder or another notification on your iPhone. It should now
   appear on your desktop.

## Integration

The CLI is designed for frontends and scripts:

```console
ancs-bridge version
ancs-bridge status
ancs-bridge doctor
ancs-bridge setup [--repair]
ancs-bridge teardown [--forget-device]
```

Successful machine commands emit one JSON object per result without requiring
an output-format flag. Setup instead streams one JSON object per line and
accepts the same format on stdin. stdout is reserved for machine data, while
diagnostics use stderr. Status and diagnostics never contain notification
titles, bodies, or app payloads. The user service starts at login only after it
has been explicitly enabled.

## Ignore notifications from selected apps

To avoid duplicates from another notification relay, add iOS bundle identifiers
to `$XDG_CONFIG_HOME/ancs-bridge/config.toml` (normally
`~/.config/ancs-bridge/config.toml`):

```toml
[notifications]
ignore_apps = ["com.apple.MobileSMS"]
```

Restart the daemon to apply changes:

```console
systemctl --user restart ancs-bridge.service
```

Matching is exact and case-sensitive, with no wildcards or normalization.
The list accepts at most 100 entries, each 1–255 UTF-8 bytes, with no Unicode
control characters or whitespace-only values. Omit the section or use an empty
list to forward all apps as before. Repair and re-pair preserve the list.

This only suppresses desktop delivery: notification content is still retrieved
from the iPhone into memory before filtering. It is not a privacy boundary.
Older binaries ignore this policy and may remove it when rewriting the config;
downgrading disables filtering.

## Troubleshooting

- **Check whether the service is running:** `systemctl --user status ancs-bridge.service`
- **Check iPhone connection and forwarding readiness:** `ancs-bridge status`
- **Check for missing prerequisites or configuration problems:** `ancs-bridge doctor`
- **Read recent errors and recovery events:** `journalctl --user-unit=ancs-bridge.service --lines=50 --no-pager`

The daemon waits for the iPhone to reconnect rather than forcing a Bluetooth
connection. For BlueZ recovery attempts, look in the journal for
`event_code=bluez-session-refresh`, `recovery_cause`, and `subscription_stage`.
These fields do not contain notification content.

If setup was interrupted by a crash or power loss, check the adapter's
`Pairable` and `Discoverable` values before retrying.

If the iPhone is paired and connected but status remains `waiting-for-services`
or reports `ancs-unavailable`, turn Bluetooth off and on again in the iPhone’s
**Settings → Bluetooth**, wait for it to reconnect, then rerun
`ancs-bridge status`.

The validated target is Arch Linux with BlueZ and a desktop notification D-Bus
session. Other adapters are experimental until they pass the same acceptance
checks.

## License

MIT. See [LICENSE](LICENSE).
