#!/usr/bin/env bash
#
# dev.sh — boots the full local dev stack in one command: the Rust BFF
# (`cargo run -p bff-api`, listening on http://127.0.0.1:3000) and the Vite
# frontend dev server (`npm run dev` in frontend/, listening on
# http://127.0.0.1:5173 by default). Both processes run concurrently with
# their output streamed to this terminal, each line prefixed with "[bff]" or
# "[web]" so it's clear which process it came from. Press Ctrl+C to stop
# both servers cleanly — the trap below kills every process in this script's
# process group, so no orphaned `cargo`/`node`/`vite` processes are left
# running. Run it from anywhere with: ./scripts/dev.sh
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Make sure `cargo` is on PATH even when this script is invoked from a
# non-login shell (e.g. an editor task runner) that hasn't sourced
# ~/.cargo/env.
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
fi

# Run this script in its own process group so we can signal every
# descendant (cargo, its child bff-api binary, npm, and its child vite
# process) at once, instead of just the direct children.
set -m

pids=()

cleanup() {
    trap - INT TERM EXIT
    echo ""
    echo "[dev] stopping..."
    for pid in "${pids[@]}"; do
        # Negative PID targets the process group rooted at that PID.
        kill -TERM "-${pid}" 2>/dev/null || kill -TERM "${pid}" 2>/dev/null
    done
    wait 2>/dev/null
    echo "[dev] stopped."
}
trap cleanup INT TERM EXIT

echo "[dev] starting bff-api (cargo run -p bff-api) on http://127.0.0.1:3000 ..."
(cd "$REPO_ROOT" && cargo run -p bff-api 2>&1 | sed -u 's/^/[bff] /') &
pids+=("$!")

echo "[dev] starting frontend (npm run dev) on http://127.0.0.1:5173 ..."
(cd "$REPO_ROOT/frontend" && npm run dev 2>&1 | sed -u 's/^/[web] /') &
pids+=("$!")

wait -n "${pids[@]}"
