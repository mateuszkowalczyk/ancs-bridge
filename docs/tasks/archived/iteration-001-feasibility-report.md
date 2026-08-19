# Iteration 001 feasibility report

## Baseline

- Date: 2026-08-19
- Host: Linux 7.1.8-arch1-3 x86_64
- BlueZ: 5.87
- Rust: 1.96.0
- Adapter: Intel Core Ultra Processors (Series 3) CNVi Bluetooth (`8086:e376`)
- Driver: `btintel_pcie`
- Controller: `hci0`, public address `4C:A9:54:EC:4B:49`, Bluetooth version 14
- Controller roles: central and peripheral
- LE support: enabled; 12 advertising instances; 1M, 2M, and Coded secondary channels
- Initial adapter state: powered, pairable, not discoverable
- iPhone model and iOS version: iPhone 15 Pro, iOS 26.6
- Material target differences: The host uses the required BlueZ 5.87 and an Intel controller. Exact controller generation may differ from earlier target assumptions.

## Reproducible procedure

The disposable implementation and commands are documented in
`spike/README.md`. Automated checks exercise event parsing, bounded fragmented
attribute reassembly, malformed input, the 64 KiB cap, and the required local
HID service shape.

## Hardware evidence

- Fresh iPhone-initiated pairing: Pass after marking the HID attributes encrypted; the temporary agent auto-confirmed only the previously verified iPhone identity address
- HID-over-GATT registration: Pass; BlueZ 5.87 accepted the disposable application on `hci0`
- HID-over-GATT pairing reliability: Pass for the Phase 0 fresh-pair, restart, and reconnect sequence on the target Intel controller
- ANCS authorization and three-characteristic discovery: Pass; iOS exposed service `7905f431-b5ce-4e99-a40f-4b1e122d00d0` and all three required characteristics after a short post-pairing delay
- Data Source subscribed before Notification Source: Pass; ordering was observed on the fresh bonded session, restart, and reconnect
- Notification attribute request and fragmented decoder: Pass after explicitly selecting a write-with-response Control Point operation; the default `bluer` operation is write without response
- One Freedesktop notification delivered: Pass
- Notification payload absent from spike persistence/logging: Pass; captured output contains UID/protocol metadata only, the executable had no system or user journal entries, and the spike creates no configuration or status files
- Restart using the existing bond without `Device1.Connect()`: Pass; ANCS resubscribed and delivered another notification
- Disconnect/reconnect and second delivery without `Device1.Connect()`: Pass; turning iPhone Bluetooth off/on caused automatic reconnect, ordered resubscription, and delivery without selecting the device
- Adapter settings and temporary BlueZ registrations restored: Pass; Pairable returned to `yes`, Discoverable returned to `no`, and advertising ActiveInstances returned to zero after cancellation and successful runs

## Phase 0 decision

Status: **Go**

- Patched BlueZ or iOS required: No
- Stable ANCS authorization unavailable: No; authorization, subscription, attribute retrieval, and delivery succeeded
- Routine reconnection requires generic `Device1.Connect()`: No; no such call exists in the spike and incoming reconnection succeeded
- HID technique unreliable on the target Intel adapter: No Phase 0 blocker observed after encrypted HID attributes were used

Production implementation may proceed to the modular daemon architecture. The
spike remains disposable and must not be promoted into production structure.

## Known build warning

`bluer` 0.17.3 builds successfully with Rust 1.96.0 but triggers Cargo's
future-incompatibility report for never-type fallback in its D-Bus calls. This
does not block the spike, but production planning should check for a fixed
upstream release or carry a narrowly reviewed patch before adopting Rust 2024.
