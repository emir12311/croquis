#!/usr/bin/env bash
# Package Croquis for Linux.
#
#   scripts/package.sh stage      build the release binary and lay out the payload
#   scripts/package.sh deb        .deb from the payload
#   scripts/package.sh arch       .pkg.tar.zst from the payload
#   scripts/package.sh rpm        .rpm from the payload       (needs rpmbuild)
#   scripts/package.sh appimage   AppImage from the payload   (needs appimagetool)
#   scripts/package.sh rpm-spec   write the spec only, without needing rpmbuild
#   scripts/package.sh appdir     lay out the AppDir only, without appimagetool
#   scripts/package.sh all        every format this machine can build
#
# The payload is the contract: each format is that one tree wrapped differently, so
# a layout that installs from one installs from all of them. `croquis-sim` looks for
# the toolchain and the libraries at `../lib/croquis/<name>` relative to the binary,
# which is exactly where the payload puts them — moving them here would mean changing
# the engine's lookup, and the two have to agree.
set -euo pipefail
cd "$(dirname "$0")/.."

APP=croquis
APP_ID=org.croquis.app
MANIFEST=spike/egui/Cargo.toml
PAYLOAD=dist/payload
DIST=dist

# Who the package says built it. Debian and Arch both require a value; neither
# requirement is about the software, so it is overridable rather than hard-coded
# to someone's address.
MAINTAINER="${MAINTAINER:-Croquis maintainers <croquis@localhost>}"
HOMEPAGE="${HOMEPAGE:-https://github.com/}"

version() {
  sed -n 's/^version = "\(.*\)"/\1/p' "$MANIFEST" | head -1
}

host_arch() {
  case "$(uname -m)" in
    x86_64) echo x86_64 ;;
    aarch64) echo aarch64 ;;
    *) uname -m ;;
  esac
}

need() {
  command -v "$1" >/dev/null || {
    echo "$1 is needed to build this format and is not installed" >&2
    exit 1
  }
}

# --- the payload every format wraps -----------------------------------------

stage() {
  local v
  v=$(version)
  [ -n "$v" ] || {
    echo "no version in $MANIFEST" >&2
    exit 1
  }
  echo "staging croquis $v for $(host_arch)"

  cargo build --release --manifest-path "$MANIFEST"
  ./scripts/build-toolchain-dist.sh

  rm -rf "$PAYLOAD"
  mkdir -p "$PAYLOAD/usr/bin" "$PAYLOAD/usr/lib/$APP"

  install -m0755 "spike/egui/target/release/$APP" "$PAYLOAD/usr/bin/$APP"
  # Stripped: this copy is what people download, and the symbols are only useful
  # to us. A failure to strip is not fatal — the package still works.
  strip --strip-unneeded "$PAYLOAD/usr/bin/$APP" 2>/dev/null || true

  # The two things the app cannot run without, in the place croquis-sim looks for
  # them: pieces of the AVR toolchain and the bundled Arduino libraries.
  cp -a .toolchain-dist "$PAYLOAD/usr/lib/$APP/toolchain"
  cp -a libraries "$PAYLOAD/usr/lib/$APP/libraries"

  install -Dm0644 "packaging/$APP_ID.desktop" \
    "$PAYLOAD/usr/share/applications/$APP_ID.desktop"
  # ImageMagick 7 renamed `convert` to `magick` and both still ship, so use
  # whichever exists rather than pinning the build to one version.
  local resize
  if command -v magick >/dev/null; then
    resize=magick
  else
    need convert
    resize=convert
  fi
  local size
  for size in 128 256 512; do
    local dir="$PAYLOAD/usr/share/icons/hicolor/${size}x${size}/apps"
    mkdir -p "$dir"
    "$resize" croquis-icon.png -resize "${size}x${size}" "$dir/$APP_ID.png"
  done

  install -Dm0644 LICENSE "$PAYLOAD/usr/share/doc/$APP/copyright"
  install -Dm0644 NOTICE "$PAYLOAD/usr/share/doc/$APP/NOTICE"
  install -Dm0644 README.md "$PAYLOAD/usr/share/doc/$APP/README.md"
  # Arch puts licences here; Debian reads them out of doc/copyright instead, so a
  # package for one distro carries a file the other ignores. It is the licence
  # text, which is not worth being clever about.
  install -Dm0644 LICENSE "$PAYLOAD/usr/share/licenses/$APP/LICENSE"

  echo "payload: $(du -sh "$PAYLOAD" | cut -f1)"
}

# --- .deb -------------------------------------------------------------------
#
# Hand-rolled rather than cargo-deb: the tree is already staged, and a .deb is an
# ar archive of three files. Building it directly keeps the payload the only thing
# that decides what is in the package.

deb() {
  need ar
  need gzip
  need xz
  [ -d "$PAYLOAD" ] || stage
  local v arch root out
  v=$(version)
  arch=$(host_arch)
  case "$arch" in
    x86_64) arch=amd64 ;;
    aarch64) arch=arm64 ;;
  esac
  # Absolute: the archive is assembled from inside two different directories, and a
  # relative path means something different in each of them.
  root="$PWD/$DIST/deb"
  out="$PWD/$DIST/${APP}_${v}_${arch}.deb"

  rm -rf "$root"
  mkdir -p "$root/DEBIAN"
  cp -a "$PAYLOAD/usr" "$root/"

  cat >"$root/DEBIAN/control" <<EOF
Package: $APP
Version: $v
Section: electronics
Priority: optional
Architecture: $arch
Installed-Size: $(du -sk "$root/usr" | cut -f1)
Depends: libc6, libx11-6, libxkbcommon0, libgl1, libwayland-client0, libasound2 | libasound2t64
Maintainer: $MAINTAINER
Homepage: $HOMEPAGE
Description: draw and simulate Arduino circuits
 Croquis is an editor for breadboard circuits: place parts, wire them to an
 Arduino Uno, write a sketch and watch it run.
EOF

  # md5sums is what dpkg uses to verify an installed package.
  (cd "$root" && find usr -type f -exec md5sum {} + | sed 's|  \./|  |' >DEBIAN/md5sums)

  (cd "$root/DEBIAN" && tar -czf "$root/control.tar.gz" --owner=0 --group=0 ./control ./md5sums)
  # xz -T0: the payload is 160 MB and single-threaded xz takes minutes on it.
  (cd "$root" && tar -cf - --owner=0 --group=0 usr | xz -T0 -6 >data.tar.xz)
  printf '2.0\n' >"$root/debian-binary"

  rm -f "$out"
  (cd "$root" && ar rc "$out" debian-binary control.tar.gz data.tar.xz)

  echo "built $(basename "$out") ($(du -h "$out" | cut -f1))"
}

# --- Arch -------------------------------------------------------------------
#
# Built by makepkg from a generated PKGBUILD that repackages the payload, rather
# than hand-written .PKGINFO + .MTREE. makepkg writes both of those, and getting
# .MTREE wrong is not something a package should fail on.

arch_pkg() {
  [ -d "$PAYLOAD" ] || stage
  local v arch out
  v=$(version)
  arch=$(host_arch)
  out="$PWD/$DIST/${APP}-${v}-1-${arch}.pkg.tar.zst"

  if command -v makepkg >/dev/null; then
    arch_pkg_makepkg "$v" "$arch" "$out"
  else
    arch_pkg_portable "$v" "$arch" "$out"
  fi
  echo "built $(basename "$out") ($(du -h "$out" | cut -f1))"
}

# On an Arch host, let makepkg do it: it writes .PKGINFO, .MTREE and the package
# itself, and what it writes is right by construction.
arch_pkg_makepkg() {
  local v=$1 arch=$2 out=$3
  local dir="$PWD/$DIST/arch-build"
  local payload_abs
  # Absolute: makepkg runs package() with $PWD inside its own srcdir, so a path
  # relative to the PKGBUILD would resolve against the wrong directory.
  payload_abs="$(cd "$PAYLOAD" && pwd)"

  rm -rf "$dir"
  mkdir -p "$dir"
  cat >"$dir/PKGBUILD" <<EOF
pkgname=$APP
pkgver=$v
pkgrel=1
pkgdesc="Draw and simulate Arduino circuits"
arch=('$arch')
url="$HOMEPAGE"
license=('GPL-3.0-only')
depends=('glibc' 'alsa-lib' 'libxkbcommon' 'mesa' 'wayland')
options=('!strip' '!debug')
package() {
  cp -a "$payload_abs/usr" "\$pkgdir/"
}
EOF

  (cd "$dir" && makepkg -f --noconfirm >/dev/null)
  mv "$dir"/*.pkg.tar.zst "$out"
}

# Anywhere else — the release builds on Ubuntu, where makepkg does not exist.
#
# A .pkg.tar.zst is a tar of the payload plus .PKGINFO and .MTREE, and bsdtar
# writes both, so one machine can build every format without an Arch container.
# This is the path CI takes; the makepkg path above is for a maintainer on Arch.
arch_pkg_portable() {
  local v=$1 arch=$2 out=$3
  need bsdtar
  need zstd
  local dir="$PWD/$DIST/arch-portable"
  local size

  rm -rf "$dir"
  mkdir -p "$dir"
  cp -a "$PAYLOAD/usr" "$dir/"
  size=$(du -sb "$dir/usr" | cut -f1)

  cat >"$dir/.PKGINFO" <<EOF
pkgname = $APP
pkgbase = $APP
pkgver = $v-1
pkgdesc = Draw and simulate Arduino circuits
url = $HOMEPAGE
builddate = $(date +%s)
packager = Croquis maintainers <croquis@localhost>
size = $size
arch = $arch
license = GPL-3.0-only
depend = glibc
depend = alsa-lib
depend = libxkbcommon
depend = mesa
depend = wayland
EOF

  # .MTREE is pacman's manifest — sizes and checksums it can verify an install
  # against. Generated last and shipped last, the way repo-add does it.
  (cd "$dir" && bsdtar --format=mtree \
    --options='!all,use-set,type,uid,gid,mode,time,size,md5,sha256,link' \
    -czf .MTREE .PKGINFO usr)
  (cd "$dir" && tar --zstd --owner=0 --group=0 --numeric-owner \
    -cf "$out" .PKGINFO usr .MTREE)
}

# --- .rpm -------------------------------------------------------------------
#
# Needs rpmbuild, which this project's dev machines may not have; CI installs it.
# The dependency names are Fedora's, because that is who installs an rpm — Debian
# users get the .deb and Arch users the package above.

rpm() {
  need rpmbuild
  local dir
  dir=$(rpm_spec)
  # Two defines that matter when the build host is not an rpm distro:
  #
  #   _dbpath          rpmbuild wants a package database, and on Arch/Artix (or
  #                    Ubuntu) /var/lib/rpm does not exist. A scratch db inside
  #                    the build dir is enough to build against — rpmbuild creates
  #                    it on first use. Do not pre-create it with `rpm --initdb`:
  #                    that segfaults under rpm 6.0.1, and the build does not need
  #                    it.
  #   __os_install_post  the default post-install runs brp-strip over everything
  #                    in the buildroot. Our payload contains the AVR toolchain's
  #                    .a archives and objects, which the host's binutils does not
  #                    recognise — and which must not be stripped anyway. The app
  #                    binary is stripped once, in stage().
  local db="$dir/.rpmdb"

  rpmbuild --define "_topdir $dir" \
    --define "_dbpath $db" \
    --define "__os_install_post %{nil}" \
    -bb "$dir/SPECS/$APP.spec" >/dev/null
  local built
  built=$(find "$dir/RPMS" -name "*.rpm" -print -quit)
  [ -n "$built" ] || {
    echo "rpmbuild produced no package" >&2
    exit 1
  }
  cp "$built" "$PWD/$DIST/"
  echo "built $(basename "$built") ($(du -h "$built" | cut -f1))"
}

# The spec on its own, so it can be written and read without rpmbuild. Echoes the
# directory rpmbuild should use as its _topdir.
rpm_spec() {
  [ -d "$PAYLOAD" ] || stage
  local v arch dir payload_abs
  v=$(version)
  arch=$(host_arch)
  dir="$PWD/$DIST/rpmbuild"
  payload_abs="$(cd "$PAYLOAD" && pwd)"

  rm -rf "$dir"
  mkdir -p "$dir/BUILD" "$dir/RPMS" "$dir/SOURCES" "$dir/SPECS" "$dir/SRPMS" "$dir/BUILDROOT"

  cat >"$dir/SPECS/$APP.spec" <<EOF
Name:           $APP
Version:        $v
Release:        1%{?dist}
Summary:        Draw and simulate Arduino circuits
License:        GPL-3.0-only
URL:            $HOMEPAGE
BuildArch:      $arch
Requires:       glibc libX11 libxkbcommon mesa-libGL alsa-lib

%description
Croquis is an editor for breadboard circuits: place parts, wire them to an
Arduino Uno, write a sketch and watch it run. The AVR toolchain and the bundled
Arduino libraries ship inside the package, so a sketch compiles without anything
else installed.

%install
mkdir -p %{buildroot}
cp -a $payload_abs/usr %{buildroot}/

%files
/usr/bin/$APP
/usr/lib/$APP
/usr/share/applications/$APP_ID.desktop
/usr/share/icons/hicolor
/usr/share/doc/$APP
/usr/share/licenses/$APP

%changelog
EOF
  echo "$dir"
}

# --- AppImage ---------------------------------------------------------------
#
# appimagetool is not something to require on a dev machine; CI fetches it. The
# AppDir is the payload again, plus the two files an AppImage needs at its root:
# the desktop entry and the icon, by name.
#
# The toolchain needs no special arrangement: croquis-sim looks beside the binary
# and then at ../lib/croquis/<name>, which is exactly where the payload already
# puts it inside the AppImage.

appimage() {
  need appimagetool
  local arch dir out
  arch=$(host_arch)
  dir=$(appdir)
  out="$PWD/$DIST/${APP}-$(version)-${arch}.AppImage"

  rm -f "$out"
  ARCH="$arch" appimagetool --no-appstream "$dir" "$out" >/dev/null
  echo "built $(basename "$out") ($(du -h "$out" | cut -f1))"
}

# The AppDir on its own. Echoes its path, so the layout can be built and inspected
# without appimagetool being installed.
appdir() {
  [ -d "$PAYLOAD" ] || stage
  local dir
  dir="$PWD/$DIST/AppDir"

  rm -rf "$dir"
  mkdir -p "$dir"
  cp -a "$PAYLOAD/usr" "$dir/"
  install -m0644 "packaging/$APP_ID.desktop" "$dir/$APP_ID.desktop"
  install -m0644 "$PAYLOAD/usr/share/icons/hicolor/256x256/apps/$APP_ID.png" \
    "$dir/$APP_ID.png"

  # AppRun resolves its own location, so the AppImage works wherever it is put
  # and from whatever directory it is launched.
  cat >"$dir/AppRun" <<'EOF'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
exec "$here/usr/bin/croquis" "$@"
EOF
  chmod 0755 "$dir/AppRun"
  echo "$dir"
}

case "${1:-stage}" in
  stage) stage ;;
  deb) deb ;;
  arch) arch_pkg ;;
  rpm) rpm ;;
  appimage) appimage ;;
  rpm-spec) rpm_spec ;;
  appdir) appdir ;;
  # The Arch package the way a non-Arch host builds it. CI takes this path, so it
  # has to be runnable by hand or it is only ever tested in CI.
  arch-portable)
    out="$PWD/$DIST/${APP}-$(version)-1-$(host_arch)-portable.pkg.tar.zst"
    arch_pkg_portable "$(version)" "$(host_arch)" "$out"
    echo "built $(basename "$out") ($(du -h "$out" | cut -f1))"
    ;;
  all)
    stage
    deb
    arch_pkg
    command -v rpmbuild >/dev/null && rpm || echo "skipped rpm: no rpmbuild"
    command -v appimagetool >/dev/null && appimage || echo "skipped appimage: no appimagetool"
    ;;
  *)
    sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
    exit 1
    ;;
esac
