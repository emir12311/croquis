/**
 * Servo — an offline servo library for Croquis.
 *
 * The signal a hobby servo expects is a 20 ms frame carrying a 544 µs (0°)
 * to 2400 µs (180°) pulse. On the Uno, Timer1 generates that directly: fast
 * PWM mode 14 (TOP is ICR1) with a prescaler of 8, so one count is 0.5 µs,
 * and ICR1 = 40000 makes the frame 20 ms long. Pointing the servo is then a
 * single write to a compare register.
 *
 * Only Timer1's two compare outputs can drive a servo this way, so `attach()`
 * accepts pins 9 (OC1A) and 10 (OC1B) and refuses the others. Note that
 * `analogWrite()` on pins 9, 10 and 11 also drives Timer1, so a sketch should
 * not use both on the same pin.
 *
 * This file is part of Croquis and is released under the same licence as the
 * rest of the project; see libraries/NOTICE.md.
 */
#include "Servo.h"

#include <avr/io.h>
#include <avr/interrupt.h>

/** One count of Timer1 here is 0.5 µs, so a register value is 2 × microseconds. */
static const uint16_t SERVO_COUNTS_PER_MICROSECOND = 2;
/** The 0° pulse, in microseconds — the classic Servo library's MIN_PULSE_WIDTH. */
static const uint16_t SERVO_MIN_PULSE_MICROSECONDS = 544;
/** The 180° pulse, in microseconds — the classic Servo library's MAX_PULSE_WIDTH. */
static const uint16_t SERVO_MAX_PULSE_MICROSECONDS = 2400;
/** ICR1 produces a 20 ms frame at a prescaler of 8: 40000 counts × 0.5 µs. */
static const uint16_t SERVO_FRAME_COUNTS = 40000;

/** Timer1 has been configured for servo pulses; shared by every Servo object. */
static bool servo_timer1_ready = false;

static void servo_setup_timer1() {
  if (servo_timer1_ready) {
    return;
  }
  // Fast PWM, mode 14: TOP is ICR1. The compare registers then hold the pulse
  // width in counts, and the frame repeats forever with no interrupt overhead.
  TCCR1A = (1 << COM1A1) | (1 << COM1B1) | (1 << WGM11);  // non-inverting, mode 14 low bits
  TCCR1B = (1 << WGM13) | (1 << WGM12) | (1 << CS11);     // mode 14 high bits, prescaler 8
  ICR1 = SERVO_FRAME_COUNTS;
  servo_timer1_ready = true;
}

/**
 * A compare register is 16 bits, and the datasheet requires the write to be
 * atomic or an interrupt can split it into a wrong high and low byte.
 */
static void servo_write_compare(uint8_t channel, uint16_t counts) {
  uint8_t sreg = SREG;
  cli();
  if (channel == 0) {
    OCR1A = counts;
  } else {
    OCR1B = counts;
  }
  SREG = sreg;
}

static uint16_t servo_angle_to_microseconds(int angle) {
  // 0° → MIN, 180° → MAX, linear in between.
  int32_t span = SERVO_MAX_PULSE_MICROSECONDS - SERVO_MIN_PULSE_MICROSECONDS;
  int32_t microseconds =
      SERVO_MIN_PULSE_MICROSECONDS + (span * (int32_t)angle) / 180;
  return (uint16_t)microseconds;
}

static void servo_output_pulse(uint8_t channel, int microseconds) {
  int clamped = microseconds;
  if (clamped < SERVO_MIN_PULSE_MICROSECONDS) {
    clamped = SERVO_MIN_PULSE_MICROSECONDS;
  }
  if (clamped > SERVO_MAX_PULSE_MICROSECONDS) {
    clamped = SERVO_MAX_PULSE_MICROSECONDS;
  }
  servo_write_compare(channel, (uint16_t)(clamped * SERVO_COUNTS_PER_MICROSECOND));
}

Servo::Servo() = default;

uint8_t Servo::attach(int pin) {
  if (pin == 9) {
    channel_ = 0;
  } else if (pin == 10) {
    channel_ = 1;
  } else {
    return 0;  // only Timer1's compare outputs can carry a servo pulse
  }
  pin_ = (uint8_t)pin;
  attached_ = true;
  pinMode(pin_, OUTPUT);
  servo_setup_timer1();
  if (microseconds_ == 0) {
    // A fresh servo parks at 0°, like the classic library.
    microseconds_ = SERVO_MIN_PULSE_MICROSECONDS;
  }
  servo_output_pulse(channel_, microseconds_);
  return 1;
}

void Servo::detach() {
  attached_ = false;
}

bool Servo::attached() const {
  return attached_;
}

void Servo::write(int angle) {
  if (angle < 0) {
    angle = 0;
  }
  if (angle > 180) {
    angle = 180;
  }
  angle_ = angle;
  microseconds_ = (int)servo_angle_to_microseconds(angle);
  if (!attached_) {
    return;
  }
  servo_output_pulse(channel_, microseconds_);
}

int Servo::read() const {
  return angle_;
}

void Servo::writeMicroseconds(int microseconds) {
  microseconds_ = microseconds;
  if (!attached_) {
    return;
  }
  servo_output_pulse(channel_, microseconds);
  // An angle in degrees is the closest place a read() after
  // writeMicroseconds() can point to.
  if (microseconds >= SERVO_MIN_PULSE_MICROSECONDS &&
      microseconds <= SERVO_MAX_PULSE_MICROSECONDS) {
    int32_t span = SERVO_MAX_PULSE_MICROSECONDS - SERVO_MIN_PULSE_MICROSECONDS;
    int32_t from_min = (int32_t)microseconds - (int32_t)SERVO_MIN_PULSE_MICROSECONDS;
    angle_ = (int)((from_min * 180 + span / 2) / span);
  }
}

int Servo::readMicroseconds() const {
  return microseconds_;
}
