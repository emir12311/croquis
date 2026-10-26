#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PRESET="${1:-$ROOT/presets/four-digit.json}"
OUT="${2:-${TMPDIR:-/tmp}/croquis-shot.png}"
DELAY="${SHOT_DELAY:-3}"

timeout 180 cargo build --manifest-path "$ROOT/spike/egui/Cargo.toml" --bin croquis-egui-spike >/dev/null

TMP="$(mktemp -d)"
export XDG_RUNTIME_DIR="$TMP/xdg"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
export WLR_BACKENDS=headless
export WLR_RENDERER=pixman
export WLR_LIBINPUT_NO_DEVICES=1

SWAY_PID=""
APP_PID=""
cleanup() {
  set +e
  if [ -n "$APP_PID" ]; then
    kill -TERM -- "-$APP_PID" 2>/dev/null
    kill -KILL -- "-$APP_PID" 2>/dev/null
  fi
  if [ -n "$SWAY_PID" ]; then
    kill -TERM -- "-$SWAY_PID" 2>/dev/null
    kill -KILL -- "-$SWAY_PID" 2>/dev/null
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

setsid sway -c "$ROOT/tools/sway-headless.conf" >"$TMP/sway.log" 2>&1 &
SWAY_PID=$!

for _ in $(seq 1 50); do
  # `-type s`: the socket and its `.lock` file share a prefix, and picking the
  # lock file gives the app a display it can never connect to.
  SOCKET="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'wayland-*' -printf '%f\n' 2>/dev/null | head -1 || true)"
  [ -n "$SOCKET" ] && break
  sleep 0.1
done
if [ -z "${SOCKET:-}" ]; then
  echo "sway did not create a Wayland socket" >&2
  cat "$TMP/sway.log" >&2
  exit 1
fi

setsid env \
  WAYLAND_DISPLAY="$SOCKET" \
  CROQUIS_UI_PRESET="$PRESET" \
  CROQUIS_UI_AUTORUN=1 \
  "$ROOT/spike/egui/target/debug/croquis-egui-spike" >"$TMP/app.log" 2>&1 &
APP_PID=$!

# A socket file is not the same as a compositor that will take clients: sway
# creates the socket a moment before it is ready, and the app launched into that
# gap dies with "failed to create display ... NoCompositor". grim only succeeds
# once a client can connect, so it doubles as the readiness probe.
for _ in $(seq 1 100); do
  timeout 5 env WAYLAND_DISPLAY="$SOCKET" grim "$TMP/probe.png" >/dev/null 2>&1 && break
  sleep 0.1
done

# Wait for the app to map a window before the fixed delay starts. The compositor
# is up long before the app has drawn anything, and a fixed sleep alone was
# grabbing a black frame on a slow start: the app registers its window here, and
# only then does the delay mean "frames have been drawn".
SWAYSOCK="$(ls "$XDG_RUNTIME_DIR"/sway-ipc.*.sock 2>/dev/null | head -1 || true)"
for _ in $(seq 1 300); do
  if [ -n "$SWAYSOCK" ] && SWAYSOCK="$SWAYSOCK" timeout 5 swaymsg -t get_tree 2>/dev/null | grep -q '"window": [0-9]'; then
    break
  fi
  if ! kill -0 "$APP_PID" 2>/dev/null; then
    echo "the app exited before it showed a window" >&2
    cat "$TMP/app.log" >&2
    exit 1
  fi
  sleep 0.1
done

sleep "$DELAY"
if ! timeout 10 env WAYLAND_DISPLAY="$SOCKET" grim "$OUT"; then
  echo "grim failed" >&2
  cat "$TMP/app.log" >&2
  cat "$TMP/sway.log" >&2
  exit 1
fi
echo "$OUT"
