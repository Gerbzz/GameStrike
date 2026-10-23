//! Rust-rendered GameStrike desktop. No HTML server, DOM or application WebView.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[path = "../platform/mod.rs"]
mod platform;

use eframe::egui;
use gamestrike::{
    application::{AppCommand, CatalogBackend, LauncherService, LibraryBackend},
    music::{MusicCommand, playback::PlaybackCommand, service::PlaybackService},
    native_capture::{DesktopOptions, isolate_capture_input, validate_pixels, validate_viewport},
    native_ui::{NativeView, boot::BootPose},
};
use std::{
    fs::OpenOptions,
    io::Write,
    path::Path,
    process::ExitCode,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

type CaptureResult = Arc<Mutex<Option<Result<(), String>>>>;

fn main() -> ExitCode {
    let options = match DesktopOptions::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("GameStrike arguments: {error}");
            return ExitCode::FAILURE;
        }
    };
    let outcome: CaptureResult = Arc::new(Mutex::new(None));
    let capture_required = options.capture.is_some();
    let result = Arc::clone(&outcome);
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("GameStrike")
            .with_app_id("PWDS.GameStrike.Native")
            .with_inner_size([options.size[0] as f32, options.size[1] as f32])
            .with_clamp_size_to_monitor_size(!capture_required)
            .with_min_inner_size([880.0, 620.0])
            .with_drag_and_drop(!options.synthetic),
        renderer: eframe::Renderer::Wgpu,
        // The custom chrome callback matches egui's single-sample/no-depth target.
        multisampling: 0,
        depth_buffer: 0,
        ..Default::default()
    };
    match eframe::run_native(
        "GameStrike",
        native_options,
        Box::new(move |cc| {
            DesktopApp::new(cc, options, result)
                .map(|app| Box::new(app) as Box<dyn eframe::App>)
                .map_err(|error| {
                    Box::<dyn std::error::Error + Send + Sync>::from(std::io::Error::other(error))
                })
        }),
    ) {
        Err(error) => {
            eprintln!("GameStrike native window: {error}");
            ExitCode::FAILURE
        }
        Ok(()) if !capture_required => ExitCode::SUCCESS,
        Ok(()) => match outcome.lock().ok().and_then(|mut result| result.take()) {
            Some(Ok(())) => ExitCode::SUCCESS,
            Some(Err(error)) => {
                eprintln!("GameStrike native capture: {error}");
                ExitCode::FAILURE
            }
            None => {
                eprintln!("Native window exited without completing capture");
                ExitCode::FAILURE
            }
        },
    }
}

struct DesktopApp {
    options: DesktopOptions,
    view: NativeView,
    service: LauncherService,
    playback: PlaybackService,
    applied_music_revision: Option<u64>,
    applied_music_replacement_revision: u64,
    started: Instant,
    skipped: bool,
    capture_frames: u8,
    capture_requested: bool,
    capture_finished: bool,
    outcome: CaptureResult,
}

impl DesktopApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        options: DesktopOptions,
        outcome: CaptureResult,
    ) -> Result<Self, String> {
        let mut view = NativeView::new(cc)?;
        if let Some(section) = options.capture_section {
            view.select_capture_section(section);
        }
        eprintln!("GameStrike renderer: {}", view.renderer_label());
        let service = if options.synthetic {
            LauncherService::synthetic()
        } else {
            let root = options
                .library
                .clone()
                .map(Ok)
                .unwrap_or_else(gamestrike::default_library_root)
                .map_err(|error| error.to_string())?;
            let ctx = cc.egui_ctx.clone();
            LauncherService::start_with_waker(
                move || {
                    LibraryBackend::open(root, platform::SystemHost)
                        .map(|backend| Box::new(backend) as Box<dyn CatalogBackend>)
                },
                Arc::new(move || ctx.request_repaint()),
            )
        };
        Ok(Self {
            options,
            view,
            service,
            // A concrete decoder/output factory is not connected yet. Never attach
            // the regression fixtures or open an audio device at app startup.
            playback: PlaybackService::unavailable(),
            applied_music_revision: None,
            applied_music_replacement_revision: 0,
            started: Instant::now(),
            skipped: false,
            capture_frames: 0,
            capture_requested: false,
            capture_finished: false,
            outcome,
        })
    }

    fn pose(&self) -> BootPose {
        let seconds = self.options.boot_time.unwrap_or_else(|| {
            if self.options.capture.is_some() {
                5.5
            } else {
                self.started.elapsed().as_secs_f32()
            }
        });
        BootPose::at(
            seconds,
            self.options.reduced_motion,
            self.skipped || self.options.skip_boot,
        )
    }

    fn poll_services(&mut self) {
        self.service.poll_events();
        self.playback.poll_events();
        if self.options.synthetic || !self.playback.snapshot().available {
            return;
        }
        let music = &self.service.snapshot().music;
        if !music.available {
            if self.applied_music_revision.is_some() {
                let _ = self.playback.dispatch(PlaybackCommand::Stop);
                match self.playback.replace_queue(&[]) {
                    Ok(()) => self.applied_music_revision = None,
                    Err(error) => self.service.report_problem(error),
                }
            }
            return;
        }
        if self.applied_music_revision == Some(music.revision) {
            return;
        }
        let revision = music.revision;
        let queue = music.queue.clone();
        let replacement = self.service.music_replacement_revision();
        let result = if self.applied_music_revision.is_none()
            || self.applied_music_replacement_revision != replacement
        {
            self.playback.replace_queue(&queue)
        } else {
            self.playback.reconcile_queue(&queue)
        };
        match result {
            Ok(()) => {
                self.applied_music_revision = Some(revision);
                self.applied_music_replacement_revision = replacement;
            }
            Err(error) => self.service.report_problem(error),
        }
    }

    fn finish_capture(&mut self, ctx: &egui::Context, result: Result<(), String>) {
        if self.capture_finished {
            return;
        }
        self.capture_finished = true;
        if let Ok(mut outcome) = self.outcome.lock() {
            *outcome = Some(result);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn capture(&mut self, ctx: &egui::Context, pose: BootPose) {
        if self.capture_finished {
            return;
        }
        let Some(path) = self.options.capture.clone() else {
            return;
        };
        if self.started.elapsed() > Duration::from_secs(20) {
            self.finish_capture(
                ctx,
                Err("Timed out before native framebuffer acceptance".into()),
            );
            return;
        }
        if self.capture_requested {
            let image = ctx.input(|input| {
                input.events.iter().find_map(|event| {
                    if let egui::Event::Screenshot { image, .. } = event {
                        Some(Arc::clone(image))
                    } else {
                        None
                    }
                })
            });
            if let Some(image) = image {
                let result = self.write_evidence(&path, &image, ctx.pixels_per_point(), pose);
                self.finish_capture(ctx, result);
            }
        } else if !self.service.snapshot().refreshing
            && self.service.snapshot().games.len() >= 8
            && self.view.content_ready()
        {
            self.capture_frames = self.capture_frames.saturating_add(1);
            if self.capture_frames >= 3 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                self.capture_requested = true;
            }
        } else {
            self.capture_frames = 0;
        }
        ctx.request_repaint_after(Duration::from_millis(30));
    }

    fn write_evidence(
        &self,
        path: &Path,
        image: &egui::ColorImage,
        scale: f32,
        pose: BootPose,
    ) -> Result<(), String> {
        let source_sha = option_env!("GAMESTRIKE_SOURCE_SHA").unwrap_or("");
        if source_sha.len() != 40 || !source_sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Capture requires GAMESTRIKE_SOURCE_SHA at compile time; no invented source identity".into());
        }
        let mut pixels = Vec::with_capacity(image.pixels.len() * 4);
        for pixel in &image.pixels {
            pixels.extend_from_slice(&pixel.to_array());
        }
        validate_viewport(image.size, self.options.size, scale)?;
        validate_pixels(image.size, &pixels, pose.settled())?;
        let [width, height] = image.size;
        let png = image::RgbaImage::from_raw(width as u32, height as u32, pixels)
            .ok_or("Could not construct native framebuffer image")?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|error| format!("Create native screenshot: {error}"))?;
        image::DynamicImage::ImageRgba8(png)
            .write_to(&mut output, image::ImageFormat::Png)
            .map_err(|error| format!("Encode native screenshot: {error}"))?;
        output.sync_all().map_err(|error| error.to_string())?;
        let metadata = serde_json::json!({
            "kind": "native Rust eframe/wgpu window framebuffer",
            "source_sha": source_sha,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "process_id": std::process::id(),
            "renderer": self.view.renderer_label(),
            "width": width, "height": height, "pixels_per_point": scale,
            "requested_logical_size": self.options.size,
            "library": "owned synthetic fixture; no real catalog or devices",
            "game_count": self.service.snapshot().games.len(),
            "section": self.view.section_name(),
            "boot_time": self.options.boot_time.unwrap_or(5.5),
            "boot_pose_seconds": pose.seconds,
            "reduced_motion": self.options.reduced_motion,
            "skip_boot": self.skipped || self.options.skip_boot,
            "readback": "paint callback acknowledged and framebuffer read back",
            "limitations": ["bundled Latin font subsets; egui fallback for other glyphs", "not Room-PC acceptance", "audio and official provider adapters not connected"],
        });
        let mut report = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path.with_extension("json"))
            .map_err(|error| format!("Create native capture metadata: {error}"))?;
        report
            .write_all(&serde_json::to_vec_pretty(&metadata).map_err(|error| error.to_string())?)
            .and_then(|()| report.sync_all())
            .map_err(|error| error.to_string())
    }
}

impl eframe::App for DesktopApp {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        isolate_capture_input(input, self.options.capture.is_some());
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_services();
        // eframe calls logic without ui while hidden. The acceptance deadline must still run.
        if self.options.capture.is_some() && !self.capture_finished {
            if self.started.elapsed() > Duration::from_secs(20) {
                self.finish_capture(
                    ctx,
                    Err("Native capture timed out while the window was hidden or not ready".into()),
                );
            } else {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_services();
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.skipped = true;
        }
        let pose = self.pose();
        self.view.set_playback(self.playback.snapshot());
        let commands = self.view.show(&ctx, self.service.snapshot(), pose);
        let mut choose_html = false;
        for command in commands {
            if matches!(&command, AppCommand::ChooseHtmlFile) {
                choose_html = true;
                continue;
            }
            if let AppCommand::Playback(command) = &command {
                let result = if self.options.synthetic {
                    Err("Synthetic capture has no playback authority".into())
                } else {
                    self.playback.dispatch(command.clone())
                };
                if let Err(error) = result {
                    self.service.report_problem(error);
                }
                continue;
            }
            if matches!(&command, AppCommand::ChooseMusicFiles) {
                if !self.options.synthetic
                    && self.service.snapshot().music.available
                    && let Some(paths) = rfd::FileDialog::new()
                        .add_filter(
                            "Local music files",
                            &["wav", "mp3", "flac", "ogg", "m4a", "aac"],
                        )
                        .pick_files()
                    && let Err(error) = self
                        .service
                        .dispatch(AppCommand::Music(MusicCommand::AddTracks(paths)))
                {
                    self.service.report_problem(error);
                }
                continue;
            }
            if let Err(error) = self.service.dispatch(command) {
                self.service.report_problem(error);
            }
        }
        if !self.options.synthetic {
            let import = pose.settled()
                && (choose_html
                    || ctx.input(|input| {
                        input.focused && input.modifiers.command && input.key_pressed(egui::Key::O)
                    }));
            if import
                && let Some(path) = rfd::FileDialog::new()
                    .add_filter("HTML games or ZIP archives", &["html", "htm", "zip"])
                    .pick_file()
                && let Err(error) = self.service.dispatch(AppCommand::Import(path))
            {
                self.service.report_problem(error);
            }
            let dropped = ctx.input(|input| {
                input
                    .raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_path_buf())
                    .collect::<Vec<_>>()
            });
            for path in dropped {
                if let Err(error) = self.service.dispatch(AppCommand::Import(path)) {
                    self.service.report_problem(error);
                }
            }
        }
        self.capture(&ctx, pose);
        if !pose.settled() && self.options.boot_time.is_none() && self.options.capture.is_none() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}

impl Drop for DesktopApp {
    fn drop(&mut self) {
        // Revoke before potentially slower renderer/catalog field destruction.
        // The service owns its own shutdown; no device or worker join is done here.
        let _ = self.playback.dispatch(PlaybackCommand::Stop);
    }
}
