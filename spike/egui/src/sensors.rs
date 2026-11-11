//! What the sketch's sensor protocols need answered.
//!
//! A DHT sensor is not a resistor: the sketch drives the data line low, lets it
//! go, and then expects forty bits timed on its own clock. Nothing in the
//! circuit model can produce that, so the app plays it back as a pulse train.

/// The pulse train a DHT22 answers with, in microseconds.
///
/// Forty bits: sixteen of humidity, sixteen of temperature, and a checksum
/// byte. Stretched from the real 50/26/70 µs timings to 80/80/120, which the
/// round trip through the engine tolerates while keeping the zero-one gap
/// unambiguous — the same stretch v0.2 uses.
pub fn dht_durations(humidity: f64, temperature: f64) -> Vec<u64> {
    let relative = (humidity * 10.0).round() as i32 & 0xffff;
    let mut scaled = (temperature.abs() * 10.0).round() as i32 & 0x7fff;
    if temperature < 0.0 {
        scaled |= 0x8000;
    }
    let bytes = [
        (relative >> 8) as u8,
        relative as u8,
        (scaled >> 8) as u8,
        scaled as u8,
    ];
    let checksum = bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte));

    let mut durations = vec![80, 80];
    for byte in bytes.into_iter().chain(std::iter::once(checksum)) {
        for bit in (0..8).rev() {
            durations.push(80);
            durations.push(if (byte >> bit) & 1 == 1 { 120 } else { 40 });
        }
    }
    durations.push(80);
    durations
}

/// Answers the sensors that need a reply within a millisecond of the sketch's
/// own edge.
///
/// This runs on the simulator's event thread, not on the UI loop: a DHT stops
/// waiting after about a millisecond, and a frame is sixteen to fifty. Its maps
/// are written by the UI thread on every evaluation and read here on every pin
/// change, so it is shared behind a lock.
#[derive(Default)]
pub struct Responder {
    /// Each ultrasonic's trigger pin: echo port, echo pin, delay, width.
    echoes: std::collections::HashMap<String, (u8, u8, u64, u64)>,
    /// Each DHT's data pin and the reading to answer with.
    dht: std::collections::HashMap<String, (f64, f64)>,
    busy: std::collections::HashMap<String, std::time::Instant>,
    /// The level last seen on each pin, for edge detection.
    levels: std::collections::HashMap<String, bool>,
    pulses: Option<croquis_sim::engine::PulseHandle>,
}

impl Responder {
    /// Start answering with the given handle, or stop when it is `None`.
    pub fn set_pulses(&mut self, pulses: Option<croquis_sim::engine::PulseHandle>) {
        self.pulses = pulses;
    }

    /// Replace what the last evaluation learned about the sensors.
    pub fn set_circuit(
        &mut self,
        echoes: std::collections::HashMap<String, (u8, u8, u64, u64)>,
        dht: std::collections::HashMap<String, (f64, f64)>,
    ) {
        self.echoes = echoes;
        self.dht = dht;
    }

    /// Forget everything, for a new circuit or a stopped simulator.
    pub fn clear(&mut self) {
        self.echoes.clear();
        self.dht.clear();
        self.busy.clear();
        self.levels.clear();
        self.pulses = None;
    }

    /// Answer a pin change, if it is one of the sensors asks for.
    pub fn on_pin(&mut self, key: &str, port: u8, pin: u8, high: bool) {
        let previous = self.levels.insert(key.to_string(), high);
        let Some(pulses) = self.pulses.clone() else {
            return;
        };
        if high {
            // A DHT is not triggered by a pulse: the sketch pulls the line low,
            // lets it go, and waits. So the answer goes out as the line rises,
            // and not again until the sensor would have finished talking. No
            // rising-edge check beyond that: `dht.begin()` reads the sensor
            // before the loop starts, and a first read has to be answered too.
            let _ = previous;
            let Some(&(humidity, temperature)) = self.dht.get(key) else {
                return;
            };
            let ready = self
                .busy
                .get(key)
                .is_none_or(|until| *until <= std::time::Instant::now());
            if ready {
                self.busy.insert(
                    key.to_string(),
                    std::time::Instant::now() + std::time::Duration::from_millis(1000),
                );
                pulses.script(port, pin, 30, dht_durations(humidity, temperature));
            }
        } else {
            // An ultrasonic answers on the falling edge of its trigger, with an
            // echo timed on the sketch's clock.
            if previous != Some(true) {
                return;
            }
            if let Some(&(echo_port, echo_pin, delay, width)) = self.echoes.get(key) {
                pulses.pulse(echo_port, echo_pin, delay, width);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_carries_two_bytes_of_each_reading_and_a_checksum() {
        let frame = dht_durations(55.0, 24.5);
        // Two start pulses, forty bits of "high duration + low duration", and
        // one closing pulse.
        assert_eq!(frame.len(), 2 + 40 * 2 + 1);

        let bits: Vec<u8> = frame[2..82]
            .chunks(2)
            .map(|pair| (pair[1] == 120) as u8)
            .collect();
        let bytes: Vec<u8> = bits
            .chunks(8)
            .map(|chunk| chunk.iter().fold(0u8, |byte, bit| (byte << 1) | bit))
            .collect();
        // 55% is 550 tenths, 24.5 C is 245 tenths.
        assert_eq!(bytes[0..2], [550u16 >> 8, 550 & 0xff].map(|b| b as u8));
        assert_eq!(bytes[2..4], [245u16 >> 8, 245 & 0xff].map(|b| b as u8));
        assert_eq!(bytes[4], bytes[..4].iter().fold(0, |sum: u8, b| sum.wrapping_add(*b)));
    }

    #[test]
    fn a_negative_temperature_is_flagged() {
        let frame = dht_durations(30.0, -10.0);
        let bits: Vec<u8> = frame[2..82]
            .chunks(2)
            .map(|pair| (pair[1] == 120) as u8)
            .collect();
        let bytes: Vec<u8> = bits
            .chunks(8)
            .map(|chunk| chunk.iter().fold(0u8, |byte, bit| (byte << 1) | bit))
            .collect();
        let raw = u16::from_be_bytes([bytes[2], bytes[3]]);
        assert_eq!(raw & 0x8000, 0x8000, "the sign bit is set");
        assert_eq!(raw & 0x7fff, 100, "and the magnitude is ten degrees");
    }
}
