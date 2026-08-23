# Iteration 009 — v0.2 setup confirmation protocol

**Status:** Completed

## Sources

- `docs/prd/02-machine-api.md`
- `docs/specs/setup-and-device-lifecycle.md`
- `docs/specs/bluetooth-accessory-pairing.md`
- `docs/tasks/current/iteration-008-v0.1.0-release-and-aur.md`

## Scope and guardrails

- This is a pre-release machine-protocol change for the next package release;
  the published immutable `v0.1.0` tag and release must not be rewritten.
- Setup has one active confirmation at a time. The caller still explicitly
  approves the displayed phone identity and passkey.
- Confirmation commands use `confirm` and `reject` without a request ID or
  boolean decision field. `cancel` remains the general setup-abort command.
- Keep stdout JSONL-only and preserve cleanup, timeout, and fail-closed
  behavior.

## Tasks

- [x] Update the approved machine API and setup/pairing specifications to
  define `confirm`, `reject`, and `cancel` without request IDs or `accept`.
- [x] Update the parser, validation, setup state machine, and regression tests
  for the simplified commands and single-active-confirmation rule.
- [x] Update the README quick start with the short confirmation line and a
  concise explanation of reject/cancel behavior.
- [x] Run focused and full automated checks, confirm no v0.1.0 tag/release was
  changed, and leave all changes uncommitted for user review.

## Implementation notes

- Setup now emits a confirmation request without a request ID and accepts only
  `{"v":1,"command":"confirm"}`, `{"v":1,"command":"reject"}`, or
  `{"v":1,"command":"cancel"}` at the corresponding lifecycle state.
- Unknown confirmation fields, including the former `requestId` and `accept`,
  are rejected as malformed input rather than ignored.
- The published `v0.1.0` tag/release was not changed. This working-tree change
  is intended for the later `v0.2.0` release.
- `cargo fmt --all -- --check`, warnings-denied Clippy, all non-hardware tests,
  and `git diff --check` pass. Hardware-only tests remain ignored as designed.
