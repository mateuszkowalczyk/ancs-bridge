# Disposable ANCS feasibility spike

This crate exists only to answer the Phase 0 questions in
`docs/prd/04-validation-release.md`. It is intentionally isolated from the
future production daemon and must be deleted or left clearly isolated after the
feasibility decision.

It never powers on Bluetooth and never calls BlueZ `Device1.Connect()`. A fresh
run temporarily changes the selected adapter's Pairable and Discoverable
properties, then restores their captured values after success, rejection,
timeout, Ctrl-C, or a returned error. Dropping the `bluer` handles unregisters
the temporary GATT application, advertisement, and agent. `SIGKILL`, power
loss, and process abort cannot run in-process cleanup; BlueZ still removes
D-Bus-owned registrations when the connection disappears, while adapter
properties may require manual restoration.

The spike logs protocol state and UIDs only. It does not log notification app,
title, or message content.

## Build and automated checks

```sh
cd spike
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```

## Read-only probe

```sh
cd spike
cargo run --locked -- probe
```

## Fresh pairing and first notification

1. Record the iPhone identity address, then remove its existing bond from both
   BlueZ and iOS.
2. Run `cargo run --locked -- fresh AA:BB:CC:DD:EE:FF` in a graphical desktop
   session. Supplying the address auto-confirms only that exact device and
   avoids the short iOS passkey deadline; omitting it uses an interactive host
   confirmation.
3. On the iPhone, open Settings > Bluetooth and select **ANCS Bridge Spike**.
   BlueZ or an iOS name cache may instead show the computer's adapter alias.
4. Confirm the passkey on the iPhone. The expected-address mode immediately
   confirms the same request on the host and rejects every other device.
5. Accept notification sharing if iOS asks. If necessary, open the paired
   device's info page and enable **Show Notifications**.
6. Send the iPhone a new notification after both ANCS subscriptions are reported.
7. Record the paired identity address printed by the spike.

The process waits up to 60 seconds for iOS to publish ANCS, then exits after
forwarding one notification. Its cleanup restores the captured Pairable and
Discoverable values.

## Restart and reconnect evidence

Use the identity address printed by the fresh run:

```sh
cargo run --locked -- reuse AA:BB:CC:DD:EE:FF
cargo run --locked -- reconnect AA:BB:CC:DD:EE:FF
```

For `reuse`, send one notification after ANCS subscribes. For `reconnect`, send
one notification, follow the prompts to turn iPhone Bluetooth off and on, then
send a second. The spike waits for incoming connections and never invokes the
generic BlueZ connect method.

## Cleanup verification

After every run, compare `bluetoothctl show` with the initial Pairable and
Discoverable values. Confirm that no **ANCS Bridge Spike** advertisement, GATT
application, or agent remains. If the process was killed before cleanup, restore
adapter state explicitly with `bluetoothctl pairable <on|off>` and
`bluetoothctl discoverable <on|off>`.

For the privacy check, send a unique canary notification, then search only the
spike's captured stdout/stderr and relevant journal slice for that canary. Do
not paste the canary into the feasibility report; record only pass/fail and the
commands used.
