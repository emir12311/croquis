#!/usr/bin/env bash
# Build the toolchain that ships inside the package.
#
# `.toolchain/` is the development tree: 277 MB, aimed at every AVR there is, and mostly
# not needed to run the app. This copies out only what Croquis uses at runtime and trims
# the rest, because the download size is the upload size on a slow connection.
#
# What is kept, and why:
#   avr-g++ / avr-gcc / avr-as / avr-ld / avr-objcopy / avr-size   the compile pipeline
#   avr/lib/gcc/avr/7.3.0/avr5                                     libgcc for the Uno's core
#   avr/avr/include                                                avr-libc headers
#   avr/avr/lib/avr5                                               avr-libc for the Uno
#   ArduinoCore-avr-1.8.6                                          the core the sketch links
#   core-cache                                                     precompiled core objects
#
# What is dropped:
#   simavr            linked statically into the binary, so the sources are not needed
#   every other AVR   avr25, avr4, avr6 and friends: we only target atmega328p
#
# Usage: scripts/build-toolchain-dist.sh    (then `npm run tauri build`)
set -euo pipefail
cd "$(dirname "$0")/.."

SRC=.toolchain
OUT=.toolchain-dist
ARCH=avr5 # atmega328p, the Uno. See boards.txt: uno.build.mcu

[ -d "$SRC" ] || { echo "no .toolchain/ — see the README's build steps" >&2; exit 1; }
rm -rf "$OUT"

# The whole bin directory, not a chosen list. Naming files individually and swallowing the
# errors with `|| true` is how the toolchain ended up without something the driver needs to
# find its own assembler — it silently fell back to the host's `as`, which answers
# "unrecognized option '-mmcu=avr5'". Seven megabytes is not worth that risk.
mkdir -p "$OUT/avr/lib" "$OUT/avr/avr"
cp -r "$SRC/avr/bin" "$OUT/avr/bin"
cp -r "$SRC/avr/include" "$OUT/avr/include"

# Only the architecture we target — but *everything else* in this directory, because it
# holds device-specs/, libgcc.a and the headers as siblings of the arch folders. Copying
# just "$ARCH" once produced a toolchain that looked complete and failed every compile with
# "device-specs/specs-atmega328p: No such file or directory".
mkdir -p "$OUT/avr/lib/gcc/avr/7.3.0"
for entry in "$SRC/avr/lib/gcc/avr/7.3.0"/*; do
  case "$(basename "$entry")" in
    avr25 | avr3 | avr31 | avr35 | avr4 | avr51 | avr6 | avrxmega*) continue ;;
  esac
  cp -r "$entry" "$OUT/avr/lib/gcc/avr/7.3.0/"
done
cp -r "$SRC/avr/libexec" "$OUT/avr/libexec" # cc1 and cc1plus live here

cp -r "$SRC/avr/avr/include" "$OUT/avr/avr/include" 2>/dev/null || true
# avr/avr/bin is not a duplicate of avr/bin: it holds the *target* tools (as, ld) that the
# host driver finds by walking ../../../../avr/bin from its own library directory. Without
# it, avr-g++ silently falls back to the host's `as`, which does not know -mmcu.
cp -r "$SRC/avr/avr/bin" "$OUT/avr/avr/bin"
mkdir -p "$OUT/avr/avr/lib"
for entry in "$SRC/avr/avr/lib"/*; do
  case "$(basename "$entry")" in
    avr25 | avr3 | avr31 | avr35 | avr4 | avr51 | avr6 | avrxmega*) continue ;;
  esac
  cp -r "$entry" "$OUT/avr/avr/lib/"
done

cp -r "$SRC/ArduinoCore-avr-1.8.6" "$OUT/ArduinoCore-avr-1.8.6"
cp -r "$SRC/core-cache" "$OUT/core-cache"

# Prove it before anyone ships it. A missing device-specs produced a toolchain that looked
# complete, packaged cleanly, and failed every compile — the app reported it as a
# compile error, which looks like the sketch is wrong.
#
# Before that, keep every dist file writable by its owner. Some copied files genuinely
# are read-only — the core's git pack files most of all — and tauri-build re-copies the
# resources on the next `cargo test` or `tauri dev` by truncating destinations that
# already exist, which fails with "Permission denied" on a read-only file. The first
# build worked, and the second never did.
chmod -R u+w "$OUT"
echo 'int main() {}' > "$OUT/.selftest.cpp"
if ! "$OUT/avr/bin/avr-g++" -mmcu=atmega328p -c "$OUT/.selftest.cpp" -o /dev/null 2>"$OUT/.selftest.err"; then
  echo "REFUSING TO SHIP: the trimmed toolchain cannot compile for atmega328p" >&2
  sed 's/^/    /' "$OUT/.selftest.err" >&2
  exit 1
fi
rm -f "$OUT/.selftest.cpp" "$OUT/.selftest.err"
echo "  self-test: avr-g++ compiles C++ for atmega328p"

echo "built $OUT"
du -sh "$SRC" "$OUT"
echo "compressed estimate: $(tar -c "$OUT" | xz -1 -T0 | wc -c | awk '{printf "%.0f MB", $1/1048576}')"
