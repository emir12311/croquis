//! The buzzer's voice: one quiet square wave per driven buzzer.
//!
//! The netlist decides whether a buzzer is across a live supply and, for a PWM
//! pin, what its frequency is. This turns that into sound the way the old app's
//! `buzzer-audio.ts` did: deliberately quiet and square, like a piezo, one voice
//! per buzzer, and the audio device is not opened until a buzzer is driven.

use std::collections::HashMap;
use std::time::Duration;

use croquis_core::netlist::OutputState;
use rodio::Source;

/// Quiet on purpose: a piezo is not a hi-fi speaker.
const MAX_GAIN: f32 = 0.035;
const MIN_HZ: f32 = 80.0;
const MAX_HZ: f32 = 4000.0;
const DEFAULT_HZ: f32 = 1000.0;
/// A square wave starts and stops with a click; ease the edge in.
const FADE_IN: Duration = Duration::from_millis(12);

/// The pitch a buzzer should sound at, clamped to what a piezo can do.
///
/// The netlist reports 0 for a driven buzzer it has no measured period for, and
/// v0.2 sounds that as 1000 Hz rather than silence.
pub fn voice_hz(frequency: Option<u16>) -> f32 {
    let hz = match frequency {
        Some(0) | None => DEFAULT_HZ,
        Some(hz) => f32::from(hz),
    };
    hz.clamp(MIN_HZ, MAX_HZ)
}

/// Which buzzers should be sounding, and at what pitch.
///
/// Split out from the audio device so the decision can be tested without one.
pub fn driven(outputs: &HashMap<String, OutputState>) -> Vec<(String, f32)> {
    let mut wanted: Vec<(String, f32)> = outputs
        .iter()
        .filter(|(_, state)| state.active && state.frequency.is_some())
        .map(|(id, state)| (id.clone(), voice_hz(state.frequency)))
        .collect();
    wanted.sort_by(|a, b| a.0.cmp(&b.0));
    wanted
}

struct Voice {
    hz: f32,
    player: rodio::Player,
}

/// Holds the output device open and a player per sounding buzzer.
pub struct BuzzerAudio {
    device: Option<rodio::MixerDeviceSink>,
    voices: HashMap<String, Voice>,
    warned: bool,
}

impl Default for BuzzerAudio {
    fn default() -> Self {
        Self {
            device: None,
            voices: HashMap::new(),
            warned: false,
        }
    }
}

impl BuzzerAudio {
    /// Bring the voices in line with the netlist: sound the driven buzzers,
    /// retune one whose pitch changed, and silence the rest.
    pub fn update(&mut self, outputs: &HashMap<String, OutputState>) {
        let wanted = driven(outputs);
        if wanted.is_empty() {
            self.stop_all();
            return;
        }
        if self.device.is_none() {
            match rodio::DeviceSinkBuilder::open_default_sink() {
                Ok(sink) => self.device = Some(sink),
                Err(error) => {
                    // Headless machines have no device. Say it once, stay quiet.
                    if !self.warned {
                        self.warned = true;
                        eprintln!("buzzer audio unavailable: {error}");
                    }
                    return;
                }
            }
        }
        self.voices
            .retain(|id, _| wanted.iter().any(|(wanted_id, _)| wanted_id == id));
        for (id, hz) in wanted {
            if let Some(voice) = self.voices.get(&id) {
                if (voice.hz - hz).abs() < 0.5 {
                    continue;
                }
                // A fixed-frequency source cannot glide, so restart it.
                self.voices.remove(&id);
            }
            let Some(device) = self.device.as_ref() else {
                return;
            };
            let player = rodio::Player::connect_new(device.mixer());
            player.append(
                rodio::source::SquareWave::new(hz)
                    .amplify(MAX_GAIN)
                    .fade_in(FADE_IN),
            );
            player.play();
            self.voices.insert(id, Voice { hz, player });
        }
    }

    /// Silence everything — the sketch stopped, or nothing is driving a buzzer.
    pub fn stop_all(&mut self) {
        for (_, voice) in self.voices.drain() {
            voice.player.stop();
        }
    }
}

impl Drop for BuzzerAudio {
    fn drop(&mut self) {
        self.stop_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buzzer(active: bool, frequency: Option<u16>) -> OutputState {
        OutputState {
            active,
            frequency,
            ..Default::default()
        }
    }

    #[test]
    fn a_driven_buzzer_without_a_measured_period_sounds_at_a_kilohertz() {
        assert_eq!(voice_hz(Some(0)), 1000.0);
        assert_eq!(voice_hz(None), 1000.0);
        assert_eq!(voice_hz(Some(440)), 440.0);
    }

    #[test]
    fn pitch_is_clamped_to_what_a_piezo_can_do() {
        assert_eq!(voice_hz(Some(1)), MIN_HZ);
        assert_eq!(voice_hz(Some(20_000)), MAX_HZ);
    }

    #[test]
    fn only_driven_buzzers_get_a_voice() {
        let mut outputs = HashMap::new();
        outputs.insert("quiet".to_string(), buzzer(false, Some(0)));
        outputs.insert("driven".to_string(), buzzer(true, Some(0)));
        outputs.insert("relay".to_string(), buzzer(true, None));
        let wanted = driven(&outputs);
        assert_eq!(wanted.len(), 1, "a relay has no frequency of its own");
        assert_eq!(wanted[0].0, "driven");
        assert_eq!(wanted[0].1, 1000.0);
    }
}
