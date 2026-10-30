#!/usr/bin/env bash
# Build a distro-agnostic AppImage for a stable Hebnix release.
#
# Reuses the canonical `make install` path (DESTDIR/PREFIX) to lay out the
# AppDir -- same install recipe as a local `make install` or the AUR
# `hebnix-linux` PKGBUILD, just staged into an AppDir instead of the real
# filesystem. linuxdeploy + its GTK plugin then bundle the shared library
# and GTK runtime-data closure; webkit2gtk's out-of-process helpers and
# their GSettings/GIO module data are bundled explicitly since linuxdeploy
# only walks the main binary's own dependency graph.
#
# Plugins/themes/config are NOT bundled here -- they are created and owned
# by the running app under $XDG_CONFIG_HOME/hebnix (see config::base_dir in
# crates/hebnix-app/src/config.rs), so replacing this AppImage never
# touches user data.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

BINARY=""
OUTPUT="Hebnix-x86_64.AppImage"
UPDATE_INFO=""

while [ $# -gt 0 ]; do
    case "$1" in
        --binary) BINARY="$2"; shift 2 ;;
        --output) OUTPUT="$2"; shift 2 ;;
        --update-info) UPDATE_INFO="$2"; shift 2 ;;
        *) echo "unknown arg: $1" >&2; exit 1 ;;
    esac
done

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
APPDIR="$WORK/AppDir"
TOOLS="$WORK/tools"
mkdir -p "$APPDIR" "$TOOLS"

if [ -z "$BINARY" ]; then
    BINARY="$REPO_ROOT/target/release/hebnix"
fi
if [ ! -x "$BINARY" ]; then
    echo "error: binary not found/executable at $BINARY (build with 'make release' first)" >&2
    exit 1
fi

echo "== staging AppDir =="
# `make install` depends on `release` (which would rebuild); we already
# have a built binary (this script's caller runs `make release` first,
# possibly in a separate CI step), so stage files the same way `make
# install` would without re-triggering the cargo build.
install -Dm755 "$BINARY" "$APPDIR/usr/bin/hebnix"
install -Dm755 "$REPO_ROOT/sidecar/hebnix-tc/hebnix-tc" "$APPDIR/usr/lib/hebnix/hebnix-tc"
install -Dm644 "$REPO_ROOT/packaging/hebnix.desktop" "$APPDIR/usr/share/applications/hebnix.desktop"
install -Dm644 "$REPO_ROOT/crates/hebnix-app/assets/hebnix.png" \
    "$APPDIR/usr/share/icons/hicolor/256x256/apps/hebnix.png"

# AppImage convention: desktop file + icon also at the AppDir root.
cp "$APPDIR/usr/share/applications/hebnix.desktop" "$APPDIR/hebnix.desktop"
cp "$APPDIR/usr/share/icons/hicolor/256x256/apps/hebnix.png" "$APPDIR/hebnix.png"
ln -sf hebnix.png "$APPDIR/.DirIcon"

echo "== fetching linuxdeploy + gtk plugin =="
LINUXDEPLOY="$TOOLS/linuxdeploy-x86_64.AppImage"
LINUXDEPLOY_GTK="$TOOLS/linuxdeploy-plugin-gtk.sh"
curl -sL -o "$LINUXDEPLOY" \
    https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
curl -sL -o "$LINUXDEPLOY_GTK" \
    https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh
chmod +x "$LINUXDEPLOY" "$LINUXDEPLOY_GTK"

echo "== bundling shared libs + GTK runtime data =="
export DEPLOY_GTK_VERSION=3
export LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-}"
(
    cd "$TOOLS"
    "$LINUXDEPLOY" --appdir "$APPDIR" \
        --executable "$APPDIR/usr/bin/hebnix" \
        --desktop-file "$APPDIR/hebnix.desktop" \
        --icon-file "$APPDIR/hebnix.png" \
        --plugin gtk
)

echo "== bundling webkit2gtk out-of-process helpers =="
# linuxdeploy only walks hebnix's own ELF dependency graph, so the
# WebKitWebProcess/WebKitNetworkProcess helper binaries (spawned at
# runtime, not linked) and their GIO/GSettings data need to be copied in
# explicitly, along with the libraries *they* need.
for libdir in /usr/lib/x86_64-linux-gnu /usr/lib; do
    if [ -d "$libdir/webkit2gtk-4.1" ]; then
        mkdir -p "$APPDIR/usr/lib/webkit2gtk-4.1"
        cp -a "$libdir/webkit2gtk-4.1/." "$APPDIR/usr/lib/webkit2gtk-4.1/"
        for helper in "$libdir"/webkit2gtk-4.1/WebKit*Process; do
            [ -e "$helper" ] || continue
            "$LINUXDEPLOY" --appdir "$APPDIR" --executable "$helper" 2>/dev/null || true
        done
        break
    fi
done
if [ -d /usr/share/glib-2.0/schemas ]; then
    mkdir -p "$APPDIR/usr/share/glib-2.0/schemas"
    cp -a /usr/share/glib-2.0/schemas/. "$APPDIR/usr/share/glib-2.0/schemas/" 2>/dev/null || true
    if command -v glib-compile-schemas &> /dev/null; then
        glib-compile-schemas "$APPDIR/usr/share/glib-2.0/schemas" || true
    fi
fi

echo "== writing AppRun wrapper =="
# Mirrors the env vars main.rs forces at startup (see main.rs comments) plus
# pointing GTK/GIO/WebKit at the bundled copies instead of the host's.
cat > "$APPDIR/AppRun" <<'EOF'
#!/usr/bin/env bash
HERE="$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)"
export GDK_BACKEND=wayland
export WEBKIT_DISABLE_COMPOSITING_MODE=1
export GIO_EXTRA_MODULES="$HERE/usr/lib/gio/modules:${GIO_EXTRA_MODULES:-}"
export GSETTINGS_SCHEMA_DIR="$HERE/usr/share/glib-2.0/schemas:${GSETTINGS_SCHEMA_DIR:-}"
export XDG_DATA_DIRS="$HERE/usr/share:${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
if [ -d "$HERE/usr/lib/webkit2gtk-4.1" ]; then
    export WEBKIT_EXEC_PATH="$HERE/usr/lib/webkit2gtk-4.1"
fi
exec "$HERE/usr/bin/hebnix" "$@"
EOF
chmod +x "$APPDIR/AppRun"

echo "== fetching appimagetool =="
APPIMAGETOOL="$TOOLS/appimagetool-x86_64.AppImage"
curl -sL -o "$APPIMAGETOOL" \
    https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
chmod +x "$APPIMAGETOOL"

echo "== packaging AppImage =="
cd "$REPO_ROOT"
if [ -n "$UPDATE_INFO" ]; then
    ARCH=x86_64 "$APPIMAGETOOL" --appimage-extract-and-run -u "$UPDATE_INFO" "$APPDIR" "$OUTPUT"
else
    ARCH=x86_64 "$APPIMAGETOOL" --appimage-extract-and-run "$APPDIR" "$OUTPUT"
fi
chmod +x "$OUTPUT"
echo "== built $OUTPUT =="
