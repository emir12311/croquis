#!/usr/bin/env bash
# Fetch the AVR toolchain into .toolchain/, which is git-ignored.
#
# This is what the README's "Build from source" section does by hand, scripted for CI and
# for anyone who would rather not copy commands. Downloads ~36 MB and compiles simavr.
set -euo pipefail
cd "$(dirname "$0")/.."

AVR_GCC_VERSION=7.3.0-atmel3.6.1-arduino7
AVR_GCC_TARBALL=avr-gcc-${AVR_GCC_VERSION}-x86_64-pc-linux-gnu.tar.bz2
AVR_GCC_SHA256=bd8c37f6952a2130ac9ee32c53f6a660feb79bee8353c8e289eb60fdcefed91e
CORE_VERSION=1.8.6

SRC=.toolchain
mkdir -p "$SRC" && cd "$SRC"

# 1. simavr — the AVR simulator. Built from source; the same make also produces the
#    libsimavr.a that build.rs links statically.
# The static library build.rs links. Checked with a glob, because the directory is named
# after the host triplet and assuming x86_64-pc-linux-gnu is how this silently produced
# nothing on a runner and failed five minutes later inside cargo.
simavr_lib() { ls simavr/simavr/obj-*/libsimavr.a 2>/dev/null | head -1; }

if [ -z "$(simavr_lib)" ]; then
  echo "==> simavr"
  [ -d simavr ] || git clone --depth 1 https://github.com/buserror/simavr
  # build-simavr, not all: the default target also builds simavr's examples, which are
  # AVR programs and need avr-gcc — the thing this script has not installed yet.
  make -C simavr -j"$(nproc)" build-simavr
fi

# Prove it, here, where the error is one line long and points at simavr.
if [ -z "$(simavr_lib)" ]; then
  echo "simavr built but produced no libsimavr.a. Found instead:" >&2
  find simavr -name "libsimavr*" -o -name "obj-*" -maxdepth 3 2>/dev/null | head -5 >&2
  echo "expected under $(pwd)/simavr/simavr/obj-*/" >&2
  exit 1
fi
echo "    libsimavr.a: $(simavr_lib)"

# 2. avr-gcc 7.3.0 — compiles the sketch. Checked against Arduino's own checksum, because
#    an unchecked 36 MB binary from the network is not something to unpack and exec.
if [ ! -x avr/bin/avr-gcc ]; then
  echo "==> avr-gcc $AVR_GCC_VERSION"
  curl -fLO "https://downloads.arduino.cc/tools/${AVR_GCC_TARBALL}"
  echo "${AVR_GCC_SHA256}  ${AVR_GCC_TARBALL}" | sha256sum -c
  mkdir -p avr
  tar xf "$AVR_GCC_TARBALL" -C avr --strip-components=1
  rm -f "$AVR_GCC_TARBALL"
fi

# 3. the Arduino core the sketch links against
if [ ! -d ArduinoCore-avr-$CORE_VERSION ]; then
  echo "==> ArduinoCore-avr $CORE_VERSION"
  git clone --depth 1 -b "$CORE_VERSION" https://github.com/arduino/ArduinoCore-avr "ArduinoCore-avr-$CORE_VERSION"
fi

# 4. precompiled core objects, so a sketch compile does not recompile the core each time.
if [ ! -d core-cache ] || [ -z "$(ls -A core-cache 2>/dev/null)" ]; then
  echo "==> core-cache"
  mkdir -p core-cache
  STUB=$(mktemp -u)
  CORE=ArduinoCore-avr-$CORE_VERSION
  # .c and .S as well as .cpp — these are all things the app would otherwise compile at run
  # time, which fails when the toolchain ships read-only (an AppImage mount, or /usr/lib owned
  # by root). wiring_pulse.S matters specifically: delay() and pulseIn() need it.
  #
  # hooks.c cannot be compiled by this toolchain: it aliases yield() to __empty, which it declares
  # `static`, and gcc 7.3 refuses an alias to a symbol with no external linkage. delay() does call
  # yield(), so the object is needed — this supplies it directly instead, weak so a sketch or
  # library can still override it.
  printf '__attribute__((weak)) void yield(void) {}\n' > "$STUB.c"
  avr/bin/avr-gcc -mmcu=atmega328p -DF_CPU=16000000L -DARDUINO=10806 -Os \
    -I"$CORE/cores/arduino" -I"$CORE/variants/standard" \
    -c "$STUB.c" -o core-cache/yield.c.o
  rm -f "$STUB.c"

  for f in "$CORE"/cores/arduino/*.c "$CORE"/cores/arduino/*.cpp "$CORE"/cores/arduino/*.S; do
    [ -e "$f" ] || continue
    case "$(basename "$f")" in hooks.c) continue ;; esac
    avr/bin/avr-g++ -mmcu=atmega328p -DF_CPU=16000000L -DARDUINO=10806 -Os \
      -I"$CORE/cores/arduino" -I"$CORE/variants/standard" \
      -c "$f" -o "core-cache/$(basename "$f").o"
  done
fi

echo
echo "toolchain ready: $(du -sh "$PWD" | cut -f1)"
avr/bin/avr-gcc --version | head -1
