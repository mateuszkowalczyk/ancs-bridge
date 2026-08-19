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

The setup command is planned for a later iteration, so developers currently
prepare this file manually. The production command itself takes no device
selection arguments:

```console
cargo run -- daemon
```

It never powers the adapter and has no generic `Device1.Connect()` recovery
path. The bonded iPhone reconnects to the retained connectable,
non-discoverable HID advertisement.

Configuration writes provided by the library are atomic and replace the file
with mode `0600`. Adapter names and bonded identity addresses are validated
before any BlueZ transport or object path is constructed.

## Read-only machine API v1

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

Successful machine commands write exactly one JSON object to stdout. Failures
leave stdout empty, write a concise diagnostic to stderr, and return nonzero.
The committed v1 fixtures live in `tests/fixtures/machine-api-v1/`.

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
