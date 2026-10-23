# Bundled libraries

The app compiles sketches offline, so it ships the Arduino libraries it
supports inside this directory. `#include <Name.h>` in a sketch resolves
against `libraries/<Name>/`, and the library's sources are compiled and
linked with the sketch.

This file exists so the provenance of everything in `libraries/` is on
record. Libraries written for Croquis are © the Croquis authors and are
released under the project's licence; libraries copied from elsewhere
carry their original notice.

| Library | Origin | Licence |
| --- | --- | --- |
| `Servo` | Written for Croquis. A minimal, offline implementation of the Arduino `Servo` API (the same class, methods and 544–2400 µs pulse widths as the version in ArduinoCore-avr's bundled libraries), driving Timer1's two compare outputs on pins 9 and 10. | Project licence |
| `Keypad` | Written for Croquis. A minimal, offline implementation of the classic Arduino `Keypad` API: a row scan with pull-up columns, debounce, and `makeKeymap`. | Project licence |
| `Stepper` | Written for Croquis. A minimal, offline implementation of the Arduino `Stepper` API using the usual four-wire, two-coil sequence and a delay derived from `setSpeed`. | Project licence |
| `LiquidCrystal` | Written for Croquis. A minimal, offline implementation of the classic Arduino `LiquidCrystal` API for a 16×2 HD44780 display in 4-bit mode; inherits `Print` and writes RS/E/D4–D7. | Project licence |
| `DHT` | Written for Croquis. A minimal, offline implementation of the Adafruit `DHT` API for DHT11/DHT22 one-wire reads (start pulse, 40-bit response, checksum, 2 s minimum interval). | Project licence |
| `IRremote` | Written for Croquis. A minimal, offline NEC receiver: the common `IRrecv`/`decode_results` polling API, backed by a pin-change interrupt and a duration buffer. | Project licence |
