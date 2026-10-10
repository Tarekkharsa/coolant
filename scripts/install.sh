#!/bin/sh
# Installs the newest Coolant into /Applications (or ~/Applications) and
# opens it:
#
#   curl -fsSL https://raw.githubusercontent.com/Tarekkharsa/coolant/main/scripts/install.sh | sh
#
# Files downloaded with curl aren't marked as downloaded from the internet,
# so macOS doesn't stop the (not notarized) app on first launch the way it
# does for a .dmg downloaded in a browser. The download is checked against
# the SHA-256 published with the release.
set -eu
REPO="Tarekkharsa/coolant"

[ "$(uname -s)" = Darwin ] || { echo "The Coolant app is for macOS; on Linux, see the README for the CLI." >&2; exit 1; }
[ "$(uname -m)" = arm64 ] || { echo "Coolant needs an Apple Silicon Mac for now." >&2; exit 1; }

# Newest release (including betas) that has a Coolant .dmg.
url=$(curl -fsSL "https://api.github.com/repos/$REPO/releases" |
  grep -o '"browser_download_url": *"[^"]*/Coolant-[^"]*\.dmg"' | head -1 | sed 's/.*"\(https:[^"]*\)"$/\1/')
[ -n "$url" ] || { echo "No Coolant .dmg found in the releases of $REPO." >&2; exit 1; }

tmp=$(mktemp -d)
cleanup() {
  hdiutil detach -quiet "$tmp/mnt" 2>/dev/null || true
  rm -rf "$tmp"
}
trap cleanup EXIT

echo "Downloading $(basename "$url")…"
curl -fsSL -o "$tmp/Coolant.dmg" "$url"
expected=$(curl -fsSL "$url.sha256" | cut -d' ' -f1)
actual=$(shasum -a 256 "$tmp/Coolant.dmg" | cut -d' ' -f1)
if [ "$expected" != "$actual" ]; then
  echo "Checksum mismatch (expected $expected, got $actual); not installing." >&2
  exit 1
fi

dest="${COOLANT_INSTALL_DIR:-/Applications}"
if [ -z "${COOLANT_INSTALL_DIR:-}" ] && [ ! -w /Applications ]; then
  dest="$HOME/Applications"
fi
mkdir -p "$dest"

# Replace a running copy cleanly.
if pgrep -f "Coolant.app/Contents/MacOS/coolant-app" >/dev/null 2>&1; then
  osascript -e 'quit app "Coolant"' >/dev/null 2>&1 || true
  sleep 1
fi

mkdir "$tmp/mnt"
hdiutil attach -quiet -nobrowse -readonly -mountpoint "$tmp/mnt" "$tmp/Coolant.dmg"
rm -rf "$dest/Coolant.app"
ditto "$tmp/mnt/Coolant.app" "$dest/Coolant.app"
echo "Installed $dest/Coolant.app"

if [ -z "${COOLANT_NO_OPEN:-}" ]; then
  open "$dest/Coolant.app"
  echo "Coolant is running: look for the thermometer in the menu bar."
fi
