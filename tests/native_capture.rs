use gamestrike::native_capture::{DesktopOptions, validate_pixels, validate_viewport};

fn options(args: &[&str]) -> Result<DesktopOptions, String> {
    DesktopOptions::parse(args.iter().map(|arg| (*arg).to_owned()))
}

#[test]
fn native_capture_requires_an_owned_synthetic_library() {
    assert!(options(&["--capture-native", "frame.png"]).is_err());
    assert!(
        options(&[
            "--capture-native",
            "frame.png",
            "--synthetic-library",
            "--library",
            "real-library"
        ])
        .is_err()
    );
    let parsed = options(&[
        "--capture-native",
        "frame.png",
        "--synthetic-library",
        "--boot-time",
        "4.5",
    ])
    .unwrap();
    assert_eq!(parsed.boot_time, Some(4.5));
    assert!(parsed.synthetic);
}

#[test]
fn intentionally_dark_opening_is_not_offered_as_automated_nonblank_evidence() {
    assert!(
        options(&[
            "--capture-native",
            "dark.png",
            "--synthetic-library",
            "--boot-time",
            "0"
        ])
        .is_err()
    );
    assert!(options(&["--synthetic-library", "--boot-time", "0"]).is_ok());
}

#[test]
fn capture_rejects_nonfinite_time_and_unbounded_dimensions() {
    for time in ["NaN", "inf", "-1", "6"] {
        assert!(options(&["--synthetic-library", "--boot-time", time]).is_err());
    }
    for width in ["0", "879", "9000"] {
        assert!(options(&["--width", width]).is_err());
    }
    assert!(options(&["--unknown"]).is_err());
}

#[test]
fn capture_rejects_monitor_clamping_and_checks_actual_dpi_extent() {
    assert!(validate_viewport([1250, 800], [1672, 941], 1.0).is_err());
    assert!(validate_viewport([1250, 800], [1250, 800], 1.5).is_err());
    assert!(validate_viewport([1875, 1200], [1250, 800], 1.5).is_ok());
    assert!(validate_viewport([2500, 1600], [1250, 800], 2.0).is_ok());
    assert!(validate_viewport([1250, 800], [1250, 800], f32::NAN).is_err());
}

#[test]
fn blank_frame_and_wrong_dimensions_fail_capture_acceptance() {
    assert!(validate_pixels([880, 620], &vec![0; 880 * 620 * 4], true).is_err());
    assert!(validate_pixels([10, 10], &[0; 400], false).is_err());
    assert!(validate_pixels([880, 620], &[255; 4], true).is_err());
}

#[test]
fn a_flat_orange_frame_is_not_a_rendered_collection() {
    let mut pixels = Vec::new();
    for _ in 0..880 * 620 {
        pixels.extend_from_slice(&[240, 120, 20, 255]);
    }
    assert!(validate_pixels([880, 620], &pixels, true).is_err());
}

#[test]
fn engineering_capture_gate_accepts_varied_original_pixels() {
    let mut pixels = Vec::new();
    for y in 0..620 {
        for x in 0..880 {
            pixels.extend_from_slice(&[
                (x % 230 + 20) as u8,
                (y % 150 + 10) as u8,
                ((x + y) % 80) as u8,
                255,
            ]);
        }
    }
    // This only tests pixel validation, not an actual GPU capture or visual fidelity.
    assert!(validate_pixels([880, 620], &pixels, true).is_ok());
    for alpha in pixels.iter_mut().skip(3).step_by(4) {
        *alpha = 1;
    }
    assert!(
        validate_pixels([880, 620], &pixels, true).is_err(),
        "nearly transparent pixels are not a visible native framebuffer"
    );
}
