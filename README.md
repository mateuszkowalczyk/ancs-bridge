# ancs-bridge

`ancs-bridge` is a distribution-neutral, read-only bridge from Apple
Notification Center Service (ANCS) to Freedesktop desktop notifications. The
production core is a Rust 2021 crate built on Tokio and `bluer`; it does not
depend on the third-party `ancs` crate.

The stable configuration and machine API are intentionally deferred. For
development, the daemon accepts explicit bonded-device inputs:

```console
cargo run -- daemon --adapter hci0 --device AA:BB:CC:DD:EE:FF
```

It never powers the adapter and has no generic `Device1.Connect()` recovery
path. The bonded iPhone reconnects to the retained connectable,
non-discoverable HID advertisement.

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
- `notification`, `clock`, and `status` provide production implementations and
  deterministic fakes.

Notification payload is held in a dedicated type that cannot be debugged,
displayed, cloned, or serialized. Status and tracing expose only state,
reason codes, UIDs, and counters. Freedesktop delivery runs on a dedicated
actor because the synchronous `notify-rust` D-Bus handle is not `Send`.

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
