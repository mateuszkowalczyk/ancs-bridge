# Iteration 003 — Runtime configuration and read-only machine API

**Status:** Completed

## Sources

- `docs/prd/01-daemon-core.md`
- `docs/prd/02-machine-api.md`
- `docs/prd/04-validation-release.md`
- `docs/specs/runtime-machine-api.md`
- `docs/tasks/archived/iteration-002-production-core.md`

## Tasks

### Configuration and daemon command

- [x] Add the version 1 configuration model and XDG path resolution, with injected path/environment boundaries for deterministic tests.
- [x] Validate the schema version, adapter name, and bonded Bluetooth identity address before any BlueZ path or transport is constructed.
- [x] Implement atomic configuration load/save support with `0600` replacement permissions and failure tests proving that the previous valid file survives.
- [x] Replace the development daemon arguments with stable `ancs-bridge daemon` configuration loading and metadata-only stderr failures.
- [x] Add tests for XDG and fallback paths, valid configuration, missing configuration, unsupported versions, malformed TOML, invalid adapters/addresses, and absence of notification fields.

### Persistent runtime status

- [x] Extend the status model to the complete machine API v1 schema with configured identity metadata, stable error codes, RFC 3339 timestamps, and daemon PID while keeping payload types structurally unrepresentable.
- [x] Implement an atomic status writer at `$XDG_RUNTIME_DIR/ancs-bridge/status.json` with `0700` directory and `0600` file permissions, including safe replacement and failure-path tests.
- [x] Integrate persistent publication at daemon startup, state/reason transitions, successful Added or Modified delivery, and error-metadata changes without allowing publication failure to stop the supervisor.
- [x] Add deterministic clock/status tests for transition timestamp stability, notification timestamps, retained last-error codes, process identity, and payload privacy canaries.

### Read-only machine commands

- [x] Implement `version --json` with the exact API-version and package-version contract.
- [x] Implement `status --json` for live and stale snapshots, including daemon liveness and configuration-identity checks without age-based false staleness.
- [x] Implement synthesized unconfigured and configured-but-not-running status results with the approved nullable fields, booleans, reason code, and stale value.
- [x] Keep stdout to exactly one JSON object on successful machine commands and empty on failure, with concise diagnostics on stderr and nonzero failure exits.
- [x] Commit golden API v1 fixtures and CLI integration tests for version, every status case, malformed/unsupported input, exit codes, stdout/stderr separation, and unknown additive status fields.

### Validation and documentation

- [x] Update developer and operator documentation for configuration paths/schema, stable daemon invocation, status/version examples, stale semantics, permissions, and privacy constraints.
- [x] Run formatting, Clippy with warnings denied, all automated tests, the Linux-relevant dependency audit, and a locked release build; record any continuing upstream compatibility exception.

## Completion notes

- All 16 iteration tasks are complete.
- All 33 unit tests and 5 machine API integration tests pass; the hardware smoke remains explicitly ignored and was not required for this behavior-only slice.
- Formatting, Clippy with warnings denied, the Linux-relevant RustSec audit, and the locked release build pass.
- The audit retains the two documented Windows-only `quick-xml` ignores, and `bluer 0.17.3` retains its documented upstream future-compatibility warning.

## Deferred work

- `doctor --json` and its setup/readiness check contract.
- Setup JSONL, the production pairing/re-pair agent, exact-device WirePlumber suppression, and teardown.
- systemd user integration, AUR packaging, clean-environment packaging tests, the wider hardware matrix, and release 0.1.0.
