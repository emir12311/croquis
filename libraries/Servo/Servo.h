/**
 * Servo — an offline servo library for Croquis.
 *
 * A minimal, self-contained implementation of the Arduino `Servo` API: the
 * same class, the same methods, the same pulse widths. It is bundled with the
 * app so a sketch can use a servo without a network connection, a package
 * manager or a copy of the Arduino IDE's libraries.
 *
 * The signal is a 20 ms frame carrying a 544 µs (0°) to 2400 µs (180°) pulse.
 * On the Uno, Timer1 drives that directly: fast PWM mode 14 (TOP is ICR1) with
 * a prescaler of 8, so one count is 0.5 µs, and ICR1 = 40000 makes the frame
 * 20 ms long. Writing the compare register sets the pulse width.
 *
 * Only Timer1's two compare outputs can drive a servo this way, so `attach()`
 * accepts pins 9 (OC1A) and 10 (OC1B) and refuses the others. Note that
 * `analogWrite()` on pins 9, 10 and 11 also drives Timer1, so a sketch should
 * not use both on the same pin.
 *
 * This file is part of Croquis and is released under the same licence as the
 * rest of the project; see libraries/NOTICE.md.
 */
#ifndef Servo_h
#define Servo_h

#include <Arduino.h>

class Servo {
public:
  Servo();

  /**
   * Attach the servo to a pin and start signalling. Returns 0 when the pin
   * cannot carry a servo pulse (only pins 9 and 10 can here).
   */
  uint8_t attach(int pin);

  /** Stop signalling. The pin keeps whatever level it had. */
  void detach();

  /** Whether the servo is attached to a pin. */
  bool attached() const;

  /** Point the servo at an angle in degrees, 0 to 180. */
  void write(int angle);

  /** The angle the servo was last told to hold. */
  int read() const;

  /** Set the pulse width directly, in microseconds. */
  void writeMicroseconds(int microseconds);

  /** The pulse width the servo was last given, in microseconds. */
  int readMicroseconds() const;

private:
  /** Which Timer1 compare output this servo uses: 0 = OC1A, 1 = OC1B. */
  uint8_t channel_ = 0;
  uint8_t pin_ = 0;
  bool attached_ = false;
  int angle_ = 0;
  int microseconds_ = 0;
};

#endif
