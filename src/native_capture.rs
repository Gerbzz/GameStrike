//! CLI and framebuffer checks shared by the real native binary and synthetic contract tests.
use std::{collections::BTreeSet, path::PathBuf};

/// A frozen capture must not depend on a hosted runner's pointer or keyboard.
/// Keep real screenshot replies and viewport metadata; ordinary input is unchanged.
#[cfg(feature = "native-ui")]
pub fn isolate_capture_input(input: &mut eframe::egui::RawInput, capturing: bool) {
    if !capturing {
        return;
    }
    input
        .events
        .retain(|event| matches!(event, eframe::egui::Event::Screenshot { .. }));
    input.events.push(eframe::egui::Event::PointerGone);
    input.hovered_files.clear();
    input.dropped_files.clear();
}

#[derive(Clone, Debug)]
pub struct DesktopOptions {
    pub library: Option<PathBuf>,
    pub synthetic: bool,
    pub capture: Option<PathBuf>,
    pub size: [u32; 2],
    pub boot_time: Option<f32>,
    pub reduced_motion: bool,
    pub skip_boot: bool,
}

impl DesktopOptions {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut result = Self {
            library: None,
            synthetic: false,
            capture: None,
            size: [1250, 800],
            boot_time: None,
            reduced_motion: false,
            skip_boot: false,
        };
        let mut args = args.into_iter();
        let mut seen = BTreeSet::new();
        while let Some(arg) = args.next() {
            if !seen.insert(arg.clone()) {
                return Err(format!("Repeated option: {arg}"));
            }
            match arg.as_str() {
                "--library" => {
                    result.library = Some(PathBuf::from(args.next().ok_or("Missing library path")?))
                }
                "--synthetic-library" => result.synthetic = true,
                "--capture-native" => {
                    result.capture =
                        Some(PathBuf::from(args.next().ok_or("Missing screenshot path")?))
                }
                "--width" | "--height" => {
                    let value = args
                        .next()
                        .ok_or("Missing window dimension")?
                        .parse::<u32>()
                        .map_err(|_| "Invalid window dimension")?;
                    let (index, minimum, maximum) = if arg == "--width" {
                        (0, 880, 3840)
                    } else {
                        (1, 620, 2160)
                    };
                    if !(minimum..=maximum).contains(&value) {
                        return Err("Window dimensions outside supported limits".into());
                    }
                    result.size[index] = value;
                }
                "--boot-time" => {
                    let value = args
                        .next()
                        .ok_or("Missing boot time")?
                        .parse::<f32>()
                        .map_err(|_| "Invalid boot time")?;
                    if !value.is_finite() || !(0.0..=5.5).contains(&value) {
                        return Err("Boot time must be finite and between 0 and 5.5 seconds".into());
                    }
                    result.boot_time = Some(value);
                }
                "--reduced-motion" => result.reduced_motion = true,
                "--skip-boot" => result.skip_boot = true,
                _ => return Err(format!("Unknown option: {arg}")),
            }
        }
        if result.synthetic && result.library.is_some() {
            return Err("Synthetic mode cannot read a real library".into());
        }
        if result.capture.is_some() && !result.synthetic {
            return Err(
                "Native capture requires --synthetic-library; personal catalogs are excluded"
                    .into(),
            );
        }
        if result.capture.is_some() && result.boot_time.is_some_and(|time| time < 0.5) {
            return Err("Automated nonblank capture supports boot poses 0.5 through 5.5; inspect the intentionally dark opening interactively".into());
        }
        if result.boot_time.is_some() && !result.synthetic {
            return Err("Frozen boot poses are restricted to synthetic acceptance mode".into());
        }
        if let Some(path) = &result.capture {
            if path.extension().is_none_or(|ext| ext != "png") {
                return Err("Native screenshot must have a .png extension".into());
            }
            if path.exists() || path.with_extension("json").exists() {
                return Err("Capture output already exists; evidence is never overwritten".into());
            }
        }
        Ok(result)
    }
}

/// Reject silently clamped windows; physical readback must match requested logical size at this DPI.
pub fn validate_viewport(
    actual: [usize; 2],
    requested: [u32; 2],
    pixels_per_point: f32,
) -> Result<(), String> {
    if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return Err("Native capture DPI is invalid".into());
    }
    for axis in 0..2 {
        let expected = requested[axis] as f64 * f64::from(pixels_per_point);
        if (actual[axis] as f64 - expected).abs() > 2.0 {
            return Err(
                "Native capture viewport was resized or clamped; requested size was not rendered"
                    .into(),
            );
        }
    }
    Ok(())
}

/// Rejects empty/flat/invalid framebuffer output. Visual comparison remains a separate human gate.
pub fn validate_pixels(
    size: [usize; 2],
    rgba: &[u8],
    collection_visible: bool,
) -> Result<(), String> {
    let [width, height] = size;
    if !(880..=7680).contains(&width)
        || !(620..=4320).contains(&height)
        || width.checked_mul(height).and_then(|n| n.checked_mul(4)) != Some(rgba.len())
    {
        return Err("Native framebuffer dimensions are invalid".into());
    }
    let mut colors = BTreeSet::new();
    let mut lit = 0;
    let mut orange = 0;
    let mut opaque = 0;
    let mut minimum = 255_u8;
    let mut maximum = 0_u8;
    // Inspect all pixels: this is a bounded one-shot acceptance operation, not a render callback.
    for pixel in rgba.as_chunks::<4>().0 {
        let [r, g, b, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
        if a < 250 {
            continue;
        }
        opaque += 1;
        let light = r.max(g).max(b);
        minimum = minimum.min(light);
        maximum = maximum.max(light);
        if light > 28 {
            lit += 1;
        }
        if r > 100 && u16::from(r) > u16::from(g) * 11 / 10 && g > b {
            orange += 1;
        }
        if colors.len() < 128 {
            colors.insert((r / 8, g / 8, b / 8));
        }
    }
    let required_lit = if collection_visible {
        width * height / 50
    } else {
        64
    };
    if opaque < width * height * 99 / 100
        || lit < required_lit
        || orange < 32
        || colors.len() < 12
        || maximum.saturating_sub(minimum) < 40
    {
        return Err("Native framebuffer is blank, flat or missing the PWDS color content".into());
    }
    Ok(())
}
