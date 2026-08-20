# ancs-bridge

`ancs-bridge` is a distribution-neutral, read-only bridge from Apple
Notification Center Service (ANCS) to Freedesktop desktop notifications. The
production core is a Rust 2021 crate built on Tokio and `bluer`; it does not
depend on the third-party `ancs` crate.

The daemon reads one versioned configuration from
`$XDG_CONFIG_HOME/ancs-bridge/config.toml`, falling back to
`~/.config/ancs-bridge/config.toml`:

```toml
schema_version = 1

[bluetooth]
adapter = "hci0"
device_address = "AA:BB:CC:DD:EE:FF"
device_name = "iPhone"

[desktop]
suppress_phone_audio = true
```

Interactive setup creates this file only after the confirmed phone reaches
ANCS readiness and all temporary BlueZ state has been cleaned up. The
production daemon itself takes no device-selection arguments:

```console
cargo run -- daemon
```

It never powers the adapter and has no generic `Device1.Connect()` recovery
path. The bonded iPhone reconnects to the retained connectable,
non-discoverable HID advertisement.

Configuration writes provided by the library are atomic and replace the file
with mode `0600`. Adapter names and bonded identity addresses are validated
before any BlueZ transport or object path is constructed.

## Diagnostics and setup

Run the seven stable environment/readiness checks before setup:

```console
ancs-bridge doctor --json
```

`doctor` always emits one complete JSON result when probes can be constructed.
Warnings cover unconfigured or transient states; any failed check makes `ok`
false and the exit status nonzero. It never powers an adapter or selects among
multiple unconfigured adapters.

Setup is a bidirectional JSON Lines protocol:

```console
ancs-bridge setup --jsonl [--disable-phone-audio] [--repair]
```

The caller reads and reacts to each flushed event before sending a command.
For example, after a `confirmation-request`, send the exact opaque request ID:

```json
{"v":1,"command":"confirm","requestId":"setup-1","accept":true}
```

The candidate, confirmation, and ANCS deadlines are five minutes, 30 seconds,
and 60 seconds. Reject, cancel, malformed or unsupported input, stdin closure,
timeout, SIGINT, and SIGTERM emit a stable final error and attempt full cleanup.
Setup reuses a unique ready bond only after confirmation. A configured but
unready bond returns `repair-required`; rerun with `--repair` to authorize
forgetting only that configured identity. Configuration is atomically written
last, and setup never calls generic `Device1.Connect()`.

SIGKILL, power loss, or a machine crash cannot run in-process cleanup. If that
happens during the pairing window, inspect `bluetoothctl show` and restore the
adapter's previous `Pairable` and `Discoverable` values before retrying. BlueZ
removes the process-owned agent, advertisement, and GATT application when its
D-Bus connection disappears.

## Machine API v1

Two stable JSON commands are available:

```console
$ cargo run --quiet -- version --json
{"apiVersion":1,"version":"0.1.0"}
$ cargo run --quiet -- status --json
{"apiVersion":1,"state":"unconfigured","reasonCode":null,"adapter":null,"deviceAddress":null,"deviceName":null,"connected":false,"servicesResolved":false,"ancsAvailable":false,"subscribed":false,"lastErrorCode":null,"lastTransitionAt":null,"lastNotificationAt":null,"pid":null,"stale":false}
```

The daemon atomically publishes owner-only runtime state at
`$XDG_RUNTIME_DIR/ancs-bridge/status.json`. `status --json` verifies the
recorded PID is a live `ancs-bridge daemon` and that the snapshot identity still
matches configuration. It preserves a stopped daemon's last snapshot with
`stale: true`; an unconfigured installation returns `unconfigured`, while a
configured installation with no snapshot returns `daemon-not-running`.

Single-result machine commands write exactly one JSON object to stdout. Setup
reserves stdout for flushed JSONL. Human diagnostics always use stderr. The
committed v1 fixtures live in `tests/fixtures/machine-api-v1/`.

## Exact-device audio suppression and teardown

`--disable-phone-audio` owns one canonical WirePlumber rule for the confirmed
identity at
`$XDG_CONFIG_HOME/wireplumber/wireplumber.conf.d/90-ancs-bridge-AA_BB_CC_DD_EE_FF.conf`.
The rule disables only `bluez_card.AA_BB_CC_DD_EE_FF`; it does not change
Bluetooth roles or affect other devices. Identical application/removal is a
no-op, while different content at the owned path is preserved and reported as
`audio-rule-conflict`. WirePlumber restarts only after a real change. A later
setup failure rolls back only a rule created by that setup transaction.

Remove bridge-owned state with:

```console
ancs-bridge teardown [--forget-device]
```

Teardown is silent and idempotent. It stops/disables the user service when the
unit exists, removes and reloads the exact audio rule, optionally forgets only
the configured bond, then deletes configuration last. Any required cleanup
failure retains configuration so the same command can be retried safely.

## systemd user service

The foreground daemon remains useful for development:

```console
./target/release/ancs-bridge daemon
```

Until the source-built AUR package is available, build and manually install
the three final artifacts from a trusted checkout:

```console
cargo build --offline --locked --release
sudo install -Dm755 target/release/ancs-bridge /usr/bin/ancs-bridge
sudo install -Dm644 LICENSE /usr/share/licenses/ancs-bridge/LICENSE
sudo install -Dm644 packaging/ancs-bridge.service /usr/lib/systemd/user/ancs-bridge.service
systemctl --user daemon-reload
```

Installation does not enable the service or change pairing/configuration. Run
setup successfully first, then explicitly enable automatic login startup:

```console
systemctl --user enable --now ancs-bridge.service
systemctl --user status --no-pager ancs-bridge.service
ancs-bridge status --json
journalctl --user-unit=ancs-bridge.service --no-pager
```

An unexpected daemon failure restarts after three seconds. Deliberate stop,
start, disable, and re-enable operations are explicit:

```console
systemctl --user stop ancs-bridge.service
systemctl --user start ancs-bridge.service
systemctl --user disable --now ancs-bridge.service
systemctl --user enable --now ancs-bridge.service
```

For bridge-owned configuration, optional audio-rule, and optional exact bond
cleanup, run `ancs-bridge teardown [--forget-device]` before uninstalling.
Manual binary/unit removal alone intentionally preserves those user resources:

```console
systemctl --user disable --now ancs-bridge.service
sudo rm -f /usr/bin/ancs-bridge
sudo rm -f /usr/lib/systemd/user/ancs-bridge.service
sudo rm -f /usr/share/licenses/ancs-bridge/LICENSE
systemctl --user daemon-reload
```

`packaging/stage-install.sh` reproduces the artifact layout under a non-root
`DESTDIR` for inspection; it intentionally refuses `/` and performs no service
or user-state changes. AUR packaging, install hooks, `.SRCINFO`, and release
publication remain deferred.

## Production modules

- `bluetooth::hid` constructs the encrypted, keyboard-shaped HID-over-GATT
  service and runtime HID/ANCS advertisement. There is no input-report send
  path.
- `bluetooth::transport` owns the BlueZ session, GATT/advertisement RAII
  handles, configured device, ANCS discovery, ordered subscriptions, and
  explicit Control Point write requests.
- `bluetooth::supervisor` reconciles the runtime state every five seconds and
  applies the 1/2/5/10/30-second BlueZ recovery backoff.
- `ancs::codec` strictly validates bounded Notification Source and incremental
  Data Source protocol data.
- `ancs::session` serializes Control Point work and owns the active session's
  UID queue, desktop handles, and app-name cache.
- `config` resolves XDG paths, validates the versioned TOML model, and performs
  atomic owner-only replacement.
- `diagnostics`, `machine`, `setup`, `audio`, `service`, and `teardown` own the
  machine protocol and transactional device lifecycle behind fakeable bounds.
- `notification`, `clock`, and `status` provide production implementations and
  deterministic fakes. Status publication adds RFC 3339 transition and
  successful-delivery timestamps without payload fields.

Notification payload is held in a dedicated type that cannot be debugged,
displayed, cloned, or serialized. Configuration, status, and tracing expose
only device metadata, state, stable reason/error codes, timestamps, UIDs, and
counters. Freedesktop delivery runs on a dedicated actor because the
synchronous `notify-rust` D-Bus handle is not `Send`.

## Automated validation

```console
cargo fmt --all -- --check
cargo clippy --offline --all-targets --all-features -- -D warnings
cargo test --offline --all-targets
cargo build --offline --locked --release
```

The RustSec audit ignores two `quick-xml` advisories that enter `Cargo.lock`
only through `notify-rust`'s Windows-only backend. The affected crate is absent
from the Linux dependency tree (`cargo tree -i quick-xml` prints nothing):

```console
cargo audit --ignore RUSTSEC-2026-0194 --ignore RUSTSEC-2026-0195
```

`bluer 0.17.3` currently emits upstream Rust future-compatibility warnings
about never-type fallback. They do not affect the Rust 2021 build. Inspect the
current compiler report with `cargo report future-incompatibilities` and
reevaluate when upgrading `bluer` or the crate edition.

## Opt-in hardware smoke

The ignored smoke test uses only production modules and requires an already
bonded/authorized iPhone. It records states and counts, never notification
content:

```console
ANCS_BRIDGE_SMOKE_ADAPTER=hci0 \
ANCS_BRIDGE_SMOKE_DEVICE=AA:BB:CC:DD:EE:FF \
RUST_LOG=info \
cargo test --test hardware_smoke \
  bonded_iphone_ready_notification_and_passive_reconnect \
  -- --ignored --nocapture --test-threads=1
```

Follow its prompts to send a notification, turn iPhone Bluetooth off and on,
and send a second notification after `ready` returns. The test has a 15-minute
overall timeout and never selects, pairs, powers, or explicitly connects a
device.

## Spike boundary

The `spike/` crate and the archived feasibility report are experimental
evidence only. Production code does not import the spike crate, and changes
should continue in the root package and PMD iterations.
