#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 || -z "$1" ]]; then
  echo "usage: ANCS_BRIDGE_BINARY=/path/to/ancs-bridge $0 DESTDIR" >&2
  echo "DESTDIR must be a non-root staging directory" >&2
  exit 2
fi

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
binary="${ANCS_BRIDGE_BINARY:-${repository_root}/target/release/ancs-bridge}"
destination="$(realpath -m -- "$1")"

if [[ "$destination" == "/" ]]; then
  echo "usage: ANCS_BRIDGE_BINARY=/path/to/ancs-bridge $0 DESTDIR" >&2
  echo "DESTDIR must be a non-root staging directory" >&2
  exit 2
fi

if [[ ! -f "$binary" || ! -x "$binary" ]]; then
  echo "ancs-bridge binary is missing or not executable: $binary" >&2
  exit 2
fi

install -Dm755 "$binary" "${destination}/usr/bin/ancs-bridge"
install -Dm644 "${repository_root}/LICENSE" \
  "${destination}/usr/share/licenses/ancs-bridge/LICENSE"
install -Dm644 "${repository_root}/packaging/ancs-bridge.service" \
  "${destination}/usr/lib/systemd/user/ancs-bridge.service"
