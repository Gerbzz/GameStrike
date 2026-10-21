//! A seekable startup clock: no wall time, random state, or accumulated deltas.

pub const BOOT_END: f32 = 5.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BootPose {
    pub seconds: f32,
    pub ignite: f32,
    pub core: f32,
    pub flash: f32,
    pub emblem: f32,
    pub fly: f32,
    pub frame: f32,
    pub content: f32,
    pub net: f32,
    pub particles: f32,
    pub shockwave: f32,
}

impl BootPose {
    pub fn at(seconds: f32, reduced_motion: bool, skipped: bool) -> Self {
        let t = if reduced_motion || skipped {
            BOOT_END
        } else if seconds.is_finite() {
            seconds.clamp(0.0, BOOT_END)
        } else {
            0.0
        };
        if t >= BOOT_END {
            return Self {
                seconds: BOOT_END, ignite: 0.0, core: 0.0, flash: 0.0,
                emblem: 1.0, fly: 1.0, frame: 1.0, content: 1.0,
                net: 1.0, particles: 0.0, shockwave: 0.0,
            };
        }
        let home = cubic(smooth(4.2, BOOT_END, t));
        Self {
            seconds: t,
            ignite: smooth(0.15, 0.5, t) * (1.0 - smooth(1.4, 2.0, t)),
            core: smooth(1.0, 2.6, t) * (1.0 - smooth(3.0, 3.35, t)),
            flash: if t < 3.0 { smooth(2.9, 3.0, t) * 0.45 }
                else { 0.75 * (-(t - 3.0) * 13.0).exp() },
            emblem: smooth(3.0, 3.45, t),
            fly: cubic(((t - 4.2) / 0.8).clamp(0.0, 1.0)),
            frame: ((t - 4.2) / 1.15).clamp(0.0, 1.0),
            content: smooth(4.55, BOOT_END, t),
            net: (smooth(1.3, 3.0, t) * 0.7 * (1.0 - home) + home).clamp(0.0, 1.0),
            particles: smooth(0.1, 0.9, t) * (1.0 - smooth(4.2, 5.4, t)),
            shockwave: smooth(3.0, 3.95, t) * (1.0 - smooth(3.5, 4.05, t)),
        }
    }

    pub fn settled(&self) -> bool {
        self.seconds >= BOOT_END && self.frame == 1.0 && self.content == 1.0
    }
}

pub(crate) fn smooth(a: f32, b: f32, value: f32) -> f32 {
    let t = ((value - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn cubic(t: f32) -> f32 {
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) * 0.5 }
}
