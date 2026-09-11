#!/usr/bin/env bash
set -euo pipefail

[[ $# -eq 1 ]] || { echo "usage: $0 <OpenConvert.AppImage|OpenConvert.deb>" >&2; exit 2; }
[[ "$(uname -s)" == "Linux" ]] || { echo "Linux is required" >&2; exit 1; }

artifact="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
[[ -f "$artifact" ]] || { echo "not found: $artifact" >&2; exit 1; }
work="$(mktemp -d "${TMPDIR:-/tmp}/openconvert-linux-smoke.XXXXXX")"
trap 'rm -rf "$work"' EXIT

echo "== checksum"
sha256sum "$artifact"

case "$artifact" in
  *.AppImage|*.appimage)
    echo "== AppImage structure"
    cp "$artifact" "$work/OpenConvert.AppImage"
    chmod +x "$work/OpenConvert.AppImage"
    (cd "$work" && ./OpenConvert.AppImage --appimage-extract >/dev/null)
    root="$work/squashfs-root"
    [[ -x "$root/AppRun" ]] || { echo "AppRun is missing" >&2; exit 1; }
    ;;
  *.deb)
    echo "== Debian package structure"
    dpkg-deb --info "$artifact"
    dpkg-deb --contents "$artifact"
    root="$work/deb-root"
    mkdir -p "$root"
    dpkg-deb --extract "$artifact" "$root"
    ;;
  *) echo "expected an AppImage or .deb" >&2; exit 2 ;;
esac

main="$(find "$root" -type f -name 'openconvert-desktop' -perm -u+x -print -quit)"
[[ -n "$main" ]] || { echo "openconvert-desktop executable is missing" >&2; exit 1; }

echo "== executable and libraries"
file "$main"
missing="$(ldd "$main" 2>/dev/null | grep 'not found' || true)"
[[ -z "$missing" ]] || { echo "$missing" >&2; exit 1; }

echo "== bundled engines"
for engine in oc-images oc-pdf oc-archive oc-audio oc-ai; do
  path="$(find "$root" -type f -name "$engine" -perm -u+x -print -quit)"
  [[ -n "$path" ]] || { echo "missing engine: $engine" >&2; exit 1; }
  file "$path"
  missing="$(ldd "$path" 2>/dev/null | grep 'not found' || true)"
  [[ -z "$missing" ]] || { echo "$missing" >&2; exit 1; }
done

if [[ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ]]; then
  echo "== GUI launch"
  "$main" >"$work/stdout.log" 2>"$work/stderr.log" &
  pid=$!
  sleep 8
  kill -0 "$pid" 2>/dev/null || { cat "$work/stderr.log" >&2; exit 1; }
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
else
  echo "SKIP: GUI launch requires DISPLAY or WAYLAND_DISPLAY"
fi

echo "Linux smoke test passed"
