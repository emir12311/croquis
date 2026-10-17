#!/usr/bin/env bash
# Build an AppImage from the .deb, without Tauri's bundler.
#
# Tauri shells out to linuxdeploy, which copies our shared libraries and then dies with no
# output at all — on this machine and on a clean GitHub runner alike. Everything linuxdeploy
# was doing we already have in the .deb, so this takes that payload, wraps it in an AppDir
# and hands it to appimagetool. No dependency scanning, no silent death.
#
# Run `npm run tauri build -- --bundles deb` first.
set -euo pipefail
cd "$(dirname "$0")/.."

DEB=$(ls src-tauri/target/release/bundle/deb/*.deb 2>/dev/null | head -1 || true)
[ -n "$DEB" ] || { echo "no .deb in src-tauri/target/release/bundle/deb — build one first" >&2; exit 1; }

VERSION=$(python3 -c "import json;print(json.load(open('src-tauri/tauri.conf.json'))['version'])")
OUTDIR=src-tauri/target/release/bundle/appimage
APPDIR=$OUTDIR/Croquis.AppDir
OUT=$OUTDIR/Croquis_${VERSION}_amd64.AppImage

rm -rf "$APPDIR" && mkdir -p "$APPDIR"
echo "==> unpacking $(basename "$DEB")"
ar p "$DEB" data.tar.gz | tar -xzf - -C "$APPDIR"

# What the AppImage runs when you launch it.
#
# An AppImage installs nothing by design, which also means it never appears in the app menu.
# This is the one piece of integration worth having: on launch it puts a desktop entry and an
# icon in the user's own ~/.local/share, no root involved. $APPIMAGE is the path to the file
# as the user has it now, so moving the AppImage corrects the entry on the next run instead of
# leaving a broken launcher behind. Nothing here fails the app: if the writes do not work, it
# still starts.
cat > "$APPDIR/AppRun" <<'RUN'
#!/bin/sh
ICONS="$HOME/.local/share/icons/hicolor/128x128/apps"
ENTRIES="$HOME/.local/share/applications"
if [ -n "${APPIMAGE:-}" ] && touch "$HOME/.local/share" 2>/dev/null; then
  mkdir -p "$ICONS" "$ENTRIES" 2>/dev/null
  cp -f "$APPDIR/croquis.png" "$ICONS/croquis.png" 2>/dev/null
  printf '%s\n' \
    '[Desktop Entry]' \
    'Type=Application' \
    'Name=Croquis' \
    'Comment=Local-first Arduino circuit simulator' \
    "Exec=\"$APPIMAGE\" %U" \
    'Icon=croquis' \
    'Terminal=false' \
    'Categories=Development;Electronics;' \
    'StartupWMClass=croquis' \
    > "$ENTRIES/croquis.desktop" 2>/dev/null
  command -v update-desktop-database >/dev/null && update-desktop-database "$ENTRIES" 2>/dev/null
fi
exec "$APPDIR/usr/bin/croquis" "$@"
RUN
chmod +x "$APPDIR/AppRun"

# appimagetool wants the desktop entry, and an icon named after Icon=, at the AppDir root.
cp "$APPDIR/usr/share/applications/Croquis.desktop" "$APPDIR/Croquis.desktop"
cp "$APPDIR/usr/share/icons/hicolor/128x128/apps/croquis.png" "$APPDIR/croquis.png"

TOOL=.toolchain/appimagetool-x86_64.AppImage
if [ ! -x "$TOOL" ]; then
  echo "==> fetching appimagetool"
  mkdir -p .toolchain
  curl -fL -o "$TOOL" \
    https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
  chmod +x "$TOOL"
fi

echo "==> packaging"
rm -f "$OUT"
# --appimage-extract-and-run, because appimagetool is itself an AppImage and FUSE is not
# available everywhere (and not in a container).
ARCH=x86_64 "$TOOL" --appimage-extract-and-run "$APPDIR" "$OUT" >/dev/null

# Prove it: the archive must contain a runnable AppRun and our binary with its toolchain.
rm -rf "${APPDIR}-check" && mkdir -p "${APPDIR}-check"
(cd "${APPDIR}-check" && "$OLDPWD/$OUT" --appimage-extract >/dev/null 2>&1)
for f in AppRun usr/bin/croquis usr/lib/Croquis/toolchain/avr/bin/avr-g++ Croquis.desktop croquis.png; do
  [ -e "${APPDIR}-check/squashfs-root/$f" ] || { echo "missing from the image: $f" >&2; exit 1; }
done
rm -rf "${APPDIR}-check"

echo
echo "built $OUT"
ls -lh "$OUT" | awk '{print "  " $5}'
