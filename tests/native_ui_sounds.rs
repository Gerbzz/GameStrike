//! Pure PCM/policy tests only. Passing these does not prove native device playback.
#[path = "../src/ui_sounds.rs"]
mod ui_sounds;

use ui_sounds::{render_cue, Cue, CuePolicy, Gate};

fn input_gate() -> Gate {
    Gate { user_input_seen: true, ..Gate::default() }
}

#[test]
fn only_actual_input_can_admit_a_short_interface_cue() {
    let mut policy = CuePolicy::default();
    assert!(!policy.request(Cue::Confirm, 0, Gate::default()));
    assert!(policy.request(Cue::Confirm, 0, input_gate()));
    assert!(!policy.request(Cue::Back, 1, input_gate()));
    assert!(policy.request(Cue::Back, 100, input_gate()));
}

#[test]
fn held_navigation_drops_repeats_and_errors_are_rate_limited() {
    let mut policy = CuePolicy::default();
    let admitted: Vec<_> = (0..1000).step_by(20)
        .filter(|&ms| policy.request(Cue::Navigate, ms, input_gate())).collect();
    assert_eq!(admitted, vec![0, 120, 240, 360, 480, 600, 720, 840, 960]);
    assert!(policy.request(Cue::Error, 1000, input_gate()));
    assert!(!policy.request(Cue::Error, 1100, input_gate()));
    assert!(!policy.request(Cue::Error, 1799, input_gate()));
    assert!(policy.request(Cue::Error, 1800, input_gate()));
}

#[test]
fn maximum_two_sources_and_all_suppression_gates_are_enforced() {
    let mut policy = CuePolicy::default();
    assert!(policy.request(Cue::Navigate, 0, input_gate()));
    assert!(policy.request(Cue::Confirm, 1, input_gate()));
    assert!(!policy.request(Cue::Error, 2, input_gate()));
    assert!(policy.request(Cue::Error, 28, input_gate()));
    for gate in [
        Gate { enabled: false, ..input_gate() },
        Gate { volume: 0.0, ..input_gate() },
        Gate { volume: f32::NAN, ..input_gate() },
        Gate { game_running: true, ..input_gate() },
        Gate { visible: false, ..input_gate() },
        Gate { capturing: true, ..input_gate() },
    ] {
        assert!(!policy.request(Cue::Confirm, 1000, gate));
    }
}

#[test]
fn pcm_is_short_bounded_finite_and_has_zero_endpoints() {
    for cue in [Cue::Navigate, Cue::Confirm, Cue::Back, Cue::Error] {
        for rate in [8000, 44100, 48000, 192000] {
            let pcm = render_cue(cue, rate, 1.0);
            assert_eq!(pcm.len(), (f64::from(rate) * cue.duration_ms() as f64 / 1000.0).round() as usize);
            assert_eq!(pcm[0], 0.0);
            assert_eq!(*pcm.last().unwrap(), 0.0);
            assert!(pcm.iter().all(|value| value.is_finite() && value.abs() <= 0.05));
            assert!(pcm.iter().any(|value| value.abs() > 0.005));
        }
        assert!(render_cue(cue, 48000, 0.0).iter().all(|value| *value == 0.0));
        assert_eq!(render_cue(cue, 48000, 2.0), render_cue(cue, 48000, 1.0));
        assert!(render_cue(cue, 48000, f32::NAN).is_empty());
        assert!(render_cue(cue, u32::MAX, 1.0).is_empty());
        assert!(render_cue(cue, 1, 1.0).is_empty());
    }
}
