//! Original interface-cue PCM and scheduling policy, with no device authority.
//! This module does not play audio. A host must apply mute/hold by stopping its
//! existing sources and submit only admitted PCM to its already chosen output.

use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    Navigate,
    Confirm,
    Back,
    Error,
}

impl Cue {
    pub fn duration_ms(self) -> u64 {
        match self {
            Self::Navigate => 28,
            Self::Confirm => 65,
            Self::Back => 45,
            Self::Error => 80,
        }
    }

    fn shape(self) -> (f64, f64, f64) {
        match self {
            Self::Navigate => (780.0, 0.032, 2.5),
            Self::Confirm => (1040.0, 0.05, 4.0),
            Self::Back => (520.0, 0.03, 3.0),
            Self::Error => (260.0, 0.036, 4.0),
        }
    }
}

/// Mono fixed-pitch pulses, matching ui/ui-sounds.js. No random or external data.
/// Invalid rates/volumes return no PCM; allocation is bounded to 80 ms at 192 kHz.
pub fn render_cue(cue: Cue, sample_rate: u32, volume: f32) -> Vec<f32> {
    if !(8_000..=192_000).contains(&sample_rate) || !volume.is_finite() {
        return Vec::new();
    }
    let volume = f64::from(volume.clamp(0.0, 1.0));
    let length = (f64::from(sample_rate) * cue.duration_ms() as f64 / 1000.0).round() as usize;
    let (frequency, peak, attack_ms) = cue.shape();
    (0..length)
        .map(|i| {
            let seconds = i as f64 / f64::from(sample_rate);
            let progress = i as f64 / (length - 1) as f64;
            let attack = ((seconds * 1000.0 / attack_ms).min(1.0) * PI / 2.0).sin();
            let phase = 2.0 * PI * frequency * seconds;
            (volume * peak * attack * (1.0 - progress).powf(2.2)
                * (phase.sin() + 0.12 * (phase * 2.0).sin()) / 1.12) as f32
        })
        .collect()
}

#[derive(Clone, Copy, Debug)]
pub struct Gate {
    pub enabled: bool,
    pub volume: f32,
    pub user_input_seen: bool,
    pub game_running: bool,
    pub visible: bool,
    pub capturing: bool,
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            enabled: true,
            volume: 0.72,
            user_input_seen: false,
            game_running: false,
            visible: true,
            capturing: false,
        }
    }
}

impl Gate {
    fn allows(self) -> bool {
        self.enabled && self.volume.is_finite() && self.volume > 0.0
            && self.user_input_seen && !self.game_running && self.visible && !self.capturing
    }
}

#[derive(Default, Debug)]
pub struct CuePolicy {
    last_navigation: Option<u64>,
    last_action: Option<u64>,
    last_error: Option<u64>,
    active_until: [Option<u64>; 2],
}

fn recent(last: Option<u64>, now_ms: u64, cooldown_ms: u64) -> bool {
    last.is_some_and(|last| now_ms.saturating_sub(last) < cooldown_ms)
}

impl CuePolicy {
    /// Discard transient reservations when the host stops its sources.
    pub fn stop(&mut self) {
        self.active_until = [None, None];
    }

    /// At most two transient voices. Suppressed requests are dropped, never queued.
    /// Monotonic milliseconds must come from the host's input/scheduling clock.
    pub fn request(&mut self, cue: Cue, now_ms: u64, gate: Gate) -> bool {
        if !gate.allows() {
            self.stop();
            return false;
        }
        let blocked = match cue {
            Cue::Navigate => recent(self.last_navigation, now_ms, 120)
                || recent(self.last_action, now_ms, 120),
            Cue::Confirm | Cue::Back => recent(self.last_action, now_ms, 100),
            Cue::Error => recent(self.last_error, now_ms, 800),
        };
        if blocked {
            return false;
        }
        for voice in &mut self.active_until {
            if voice.is_some_and(|until| until <= now_ms) {
                *voice = None;
            }
        }
        let Some(slot) = self.active_until.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        *slot = Some(now_ms.saturating_add(cue.duration_ms()));
        match cue {
            Cue::Navigate => self.last_navigation = Some(now_ms),
            Cue::Confirm | Cue::Back => self.last_action = Some(now_ms),
            Cue::Error => self.last_error = Some(now_ms),
        }
        true
    }
}
