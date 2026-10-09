#!/bin/sh
# Saves a PNG of coolant-app as it renders, straight from the GPU.
#
#   scripts/screenshot.sh [out.png] [seconds]   (default docs/img/dashboard.png, 25)
#
# Window capture APIs need screen-recording permission and return blank
# images for Metal windows on recent macOS, so this builds the app against a
# copy of GPUI patched (scripts/gpui-capture.patch) to write each frame to a
# BMP and to keep rendering while the window is hidden. Nothing in the repo
# or the normal build changes.
set -eu
REPO="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$REPO/docs/img/dashboard.png}"
SECS="${2:-25}"
WORK="${TMPDIR:-/tmp}/coolant-screenshot"
GPUI_SRC="$(ls -d "$HOME"/.cargo/registry/src/*/gpui-0.2.2 | head -1)"

mkdir -p "$WORK"
rm -rf "$WORK/gpui"
cp -R "$GPUI_SRC" "$WORK/gpui"
chmod -R u+w "$WORK/gpui"
patch -s -d "$WORK/gpui" -p1 <"$REPO/scripts/gpui-capture.patch"

cp "$REPO/Cargo.lock" "$WORK/Cargo.lock.bak"
trap 'cp "$WORK/Cargo.lock.bak" "$REPO/Cargo.lock"' EXIT
(cd "$REPO" && CARGO_TARGET_DIR="$WORK/target" cargo build -q -p coolant-app \
  --config "patch.crates-io.gpui.path=\"$WORK/gpui\"")

rm -f "$WORK/frame.bmp"
GPUI_CAPTURE="$WORK/frame.bmp" COOLANT_SCREENSHOT_SECS="$SECS" "$WORK/target/debug/coolant-app" >/dev/null 2>&1
sips -s format png "$WORK/frame.bmp" --out "$OUT" >/dev/null
echo "saved $OUT"
