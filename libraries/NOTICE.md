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
