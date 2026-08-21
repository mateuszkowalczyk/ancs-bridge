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

## Quick start

1. Open a terminal and install `ancs-bridge`:

   ```console
   yay -S ancs-bridge
   ```

2. Before setup, remove any existing pairing between the iPhone and computer
   on both devices. On the iPhone, tap the information button next to
   **omarchy**, then tap **Forget This Device**. On the computer, remove the
   iPhone from the list of paired Bluetooth devices.

3. Start setup and leave the terminal open:

   ```console
   ancs-bridge setup
   ```

4. On your iPhone, open **Settings → Bluetooth**.

5. Under **Other Devices**, tap **omarchy**. Always start a new pairing from
   the iPhone, not from the computer's Bluetooth settings.

6. When the terminal shows a `confirmation-request`, check that its phone and
   pairing code match your iPhone. Copy its `requestId`, replace
   `PASTE_REQUEST_ID_HERE` below, and paste the completed line into the same
   terminal:

   ```json
   {"v":1,"command":"confirm","requestId":"PASTE_REQUEST_ID_HERE","accept":true}
   ```

7. Wait until the terminal shows `"event":"complete"`.

8. Start the notification service and make it start automatically when you log
   in:

   ```console
   systemctl --user enable --now ancs-bridge.service
   ```

9. Wait a few seconds, then check that the service reports `"state":"ready"`:

   ```console
   ancs-bridge status
   ```

10. Create a reminder or another notification on your iPhone. It should now
   appear on your desktop.

The package only installs the program and service file. It does not pair a
phone, change your settings, or start the service without your confirmation.

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

## Troubleshooting

```console
systemctl --user status ancs-bridge.service
journalctl --user-unit=ancs-bridge.service --no-pager
ancs-bridge doctor
ancs-bridge status
```

The daemon does not force Bluetooth power or use generic Bluetooth connection
attempts for routine recovery. If setup is interrupted by a crash or power
loss, check the adapter’s `Pairable` and `Discoverable` values before retrying.

The validated target is Arch Linux with BlueZ and a desktop notification D-Bus
session. Other adapters are experimental until they pass the same acceptance
checks.

## Development

The source tree includes the systemd user unit. Common development checks are:

```console
cargo fmt --all -- --check
cargo clippy --offline --locked --all-targets --all-features -- -D warnings
cargo test --offline --locked --all-targets
cargo build --offline --locked --release
```

The package installs only the daemon, MIT license, and user unit. It never
enables services, pairs devices, changes user configuration, or restarts
WirePlumber during installation. Use `ancs-bridge teardown` before removing
bridge-owned configuration, audio rules, or the optional bond.

## License

MIT. See [LICENSE](LICENSE).
