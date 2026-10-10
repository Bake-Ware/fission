#!/usr/bin/env bash
# Launch fission-app in a demo state, screenshot its window, and close it.
#   tools/snap.sh out/name.png "edit:3,view:front" [design.fission]
set -euo pipefail
out="$(realpath -m "$1")"; demo="${2:-}"; shift 2 || true
root="$(cd "$(dirname "$0")/.." && pwd)"
FISSION_DEMO="$demo" "$root/target/release/fission-app" "$@" >/dev/null 2>&1 &
pid=$!
trap 'kill $pid 2>/dev/null || true' EXIT
sleep "${SNAP_DELAY:-3}"
spectacle -b -n -a -o "$out"
echo "$out"
