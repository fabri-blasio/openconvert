#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 [--allow-unsigned] <OpenConvert.dmg>" >&2
  exit 2
}

allow_unsigned=0
if [[ "${1:-}" == "--allow-unsigned" ]]; then
  allow_unsigned=1
  shift
fi
[[ $# -eq 1 ]] || usage
[[ "$(uname -s)" == "Darwin" ]] || { echo "macOS is required" >&2; exit 1; }

dmg="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
[[ -f "$dmg" ]] || { echo "not found: $dmg" >&2; exit 1; }

work="$(mktemp -d "${TMPDIR:-/tmp}/openconvert-macos-smoke.XXXXXX")"
mount="$work/mount"
app_copy="$work/OpenConvert.app"
mounted=0
cleanup() {
  if [[ $mounted -eq 1 ]]; then hdiutil detach "$mount" -quiet || true; fi
  rm -rf "$work"
}
trap cleanup EXIT

echo "== checksum"
shasum -a 256 "$dmg"
echo "== disk image"
hdiutil verify "$dmg"
mkdir -p "$mount"
hdiutil attach "$dmg" -readonly -nobrowse -mountpoint "$mount" -quiet
mounted=1

app="$(find "$mount" -maxdepth 1 -type d -name '*.app' -print -quit)"
[[ -n "$app" ]] || { echo "no .app bundle in disk image" >&2; exit 1; }
main="$app/Contents/MacOS/openconvert-desktop"
[[ -x "$main" ]] || { echo "missing executable: $main" >&2; exit 1; }

echo "== architecture"
file "$main"
case "$(uname -m)" in
  arm64) file "$main" | grep -qE 'arm64|universal' || { echo "not an Apple Silicon build" >&2; exit 1; } ;;
  x86_64) file "$main" | grep -qE 'x86_64|universal' || { echo "not an Intel build" >&2; exit 1; } ;;
esac

echo "== bundled engines"
engine_dir="$app/Contents/Resources/engines"
for engine in oc-images oc-pdf oc-archive oc-audio oc-ai; do
  [[ -x "$engine_dir/$engine" ]] || { echo "missing engine: $engine_dir/$engine" >&2; exit 1; }
  file "$engine_dir/$engine"
done

echo "== dynamic libraries"
while IFS= read -r binary; do
  otool -L "$binary"
done < <(printf '%s\n' "$main" "$engine_dir"/oc-*)

echo "== signature and Gatekeeper"
if [[ $allow_unsigned -eq 1 ]]; then
  codesign --verify --deep --strict --verbose=4 "$app" || echo "WARNING: unsigned development artifact"
  spctl --assess --type execute --verbose=4 "$app" || echo "WARNING: Gatekeeper rejects the unsigned development artifact"
else
  codesign --verify --deep --strict --verbose=4 "$app"
  spctl --assess --type execute --verbose=4 "$app"
  xcrun stapler validate "$dmg"
fi

echo "== launch from a disposable copy"
ditto "$app" "$app_copy"
if [[ $allow_unsigned -eq 1 ]]; then
  xattr -dr com.apple.quarantine "$app_copy"
fi
open -na "$app_copy"
sleep 8
pgrep -f "$app_copy/Contents/MacOS/openconvert-desktop" >/dev/null || {
  echo "OpenConvert did not remain running" >&2
  exit 1
}
pkill -f "$app_copy/Contents/MacOS/openconvert-desktop" || true
echo "macOS smoke test passed"
