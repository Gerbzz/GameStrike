//! Trusted local library UI. Imported HTML is never loaded in this WebView.
#[cfg(not(windows))]
fn main() {
    eprintln!("The GameStrike visual library currently requires Windows.");
}

#[cfg(windows)]
mod windows_launcher {
    use gamestrike::{ImportOutcome, Library};
    use serde::Deserialize;
    use serde_json::{Value, json};
    use std::{
        borrow::Cow,
        env,
        error::Error,
        ffi::OsString,
        fs,
        mem::size_of,
        os::windows::ffi::OsStringExt,
        path::PathBuf,
        process::{Command, ExitStatus},
        sync::Arc,
        thread,
        time::Duration,
    };
    use tao::{
        dpi::LogicalSize,
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
        platform::windows::WindowExtWindows,
        window::WindowBuilder,
    };
    use uuid::Uuid;
    use webview2_com::{
        CapturePreviewCompletedHandler,
        Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
    };
    use windows::Win32::System::Com::{STREAM_SEEK_SET, StructuredStorage::CreateStreamOnHGlobal};
    use windows_sys::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
        OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };
    use wry::{
        WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows,
        http::{Request, Response, StatusCode, header},
    };

    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum CommandMessage {
        Ready,
        PickAndImport,
        LaunchGame { id: String, allow_network: bool },
    }

    enum UiEvent {
        Command(CommandMessage),
        CapturePreview,
        CaptureComplete(std::result::Result<(), String>),
        ImportResult(std::result::Result<ImportOutcome, String>),
        PlayerExited {
            id: Uuid,
            result: std::result::Result<ExitStatus, String>,
        },
    }

    #[derive(Clone, Copy)]
    enum CaptureView {
        Library,
        LongTitle,
        Launch,
        About,
    }

    pub fn run() -> Result<(), Box<dyn Error>> {
        let (root, mut capture_path, window_size, capture_view) = launch_options()?;
        let library = Arc::new(Library::open(&root)?);
        let root = fs::canonicalize(root)?;
        let event_loop = EventLoopBuilder::<UiEvent>::with_user_event().build();
        let window = WindowBuilder::new()
            .with_title("GameStrike")
            .with_inner_size(window_size)
            .with_min_inner_size(LogicalSize::new(880.0, 620.0))
            .build(&event_loop)?;
        let proxy = event_loop.create_proxy();
        let background_proxy = event_loop.create_proxy();
        let webview = WebViewBuilder::new()
            .with_custom_protocol("gamestrikeui".into(), |_, request| serve_ui(request))
            .with_https_scheme(true)
            .with_url("gamestrikeui://localhost/index.html")
            .with_navigation_handler(|url| url == "about:blank" || is_ui_origin(&url))
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .with_drag_drop_handler(|_| true)
            .with_default_context_menus(false)
            .with_devtools(false)
            .with_ipc_handler(move |request| {
                if !is_ui_origin(&request.uri().to_string()) || request.body().len() > 512 {
                    return;
                }
                if let Ok(command) = serde_json::from_str::<CommandMessage>(request.body()) {
                    let _ = proxy.send_event(UiEvent::Command(command));
                }
            })
            .build(&window)?;

        let mut view_prepared = false;
        event_loop.run(move |event, _, flow| {
            let _keep_alive = (&window, &webview);
            *flow = ControlFlow::Wait;
            match event {
                Event::UserEvent(UiEvent::Command(CommandMessage::Ready)) => {
                    send_library(&webview, &library);
                    if capture_path.is_some() {
                        let proxy = background_proxy.clone();
                        thread::spawn(move || {
                            thread::sleep(Duration::from_millis(1600));
                            let _ = proxy.send_event(UiEvent::CapturePreview);
                        });
                    }
                }
                Event::UserEvent(UiEvent::CapturePreview) => {
                    let prepare = if view_prepared {
                        None
                    } else {
                        match capture_view {
                            CaptureView::Library => None,
                            CaptureView::LongTitle => Some("const card = document.querySelector('.game-card:last-child'); card?.focus(); card?.scrollIntoView({ block: 'nearest', inline: 'nearest' })"),
                            CaptureView::Launch => {
                                Some("document.querySelector('.game-card')?.click()")
                            }
                            CaptureView::About => {
                                Some("document.getElementById('about-button').click()")
                            }
                        }
                    };
                    if let Some(script) = prepare {
                        view_prepared = true;
                        if let Err(error) = webview.evaluate_script(script) {
                            eprintln!("Could not prepare launcher preview: {error}");
                            std::process::exit(2);
                        }
                        let proxy = background_proxy.clone();
                        thread::spawn(move || {
                            thread::sleep(Duration::from_millis(350));
                            let _ = proxy.send_event(UiEvent::CapturePreview);
                        });
                    } else if let Some(path) = capture_path.take() {
                        if let Err(error) = capture_preview(&webview, path, background_proxy.clone()) {
                            eprintln!("Could not capture launcher: {error}");
                            std::process::exit(2);
                        }
                    }
                }
                Event::UserEvent(UiEvent::CaptureComplete(result)) => {
                    match result {
                        Ok(()) => println!("Launcher preview captured."),
                        Err(error) => {
                            eprintln!("Could not capture launcher: {error}");
                            std::process::exit(2);
                        }
                    }
                    *flow = ControlFlow::Exit;
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::PickAndImport)) => {
                    match pick_game_file(window.hwnd()) {
                        Ok(Some(path)) => {
                            let name = path
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "game".to_owned());
                            send(&webview, json!({"type":"importing", "name": name}));
                            let library = Arc::clone(&library);
                            let proxy = background_proxy.clone();
                            thread::spawn(move || {
                                let result = library.import_with_result(path).map_err(|error| error.to_string());
                                let _ = proxy.send_event(UiEvent::ImportResult(result));
                            });
                        }
                        Ok(None) => {}
                        Err(error) => send(&webview, json!({"type":"error", "message":error})),
                    }
                }
                Event::UserEvent(UiEvent::ImportResult(result)) => {
                    match result {
                        Ok(outcome) => send(&webview, json!({"type":"imported", "game":outcome.game, "already_present":outcome.already_present})),
                        Err(error) => send(
                            &webview,
                            json!({"type":"error", "message":format!("Import rejected: {error}")}),
                        ),
                    }
                    send_library(&webview, &library);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::LaunchGame {
                    id,
                    allow_network,
                })) => match launch(&library, &root, &id, allow_network) {
                    Ok(mut child) => {
                        let id = Uuid::parse_str(&id).expect("validated by launch");
                        send(&webview, json!({"type":"player_started", "id":id}));
                        let proxy = background_proxy.clone();
                        thread::spawn(move || {
                            let result = child.wait().map_err(|error| error.to_string());
                            let _ = proxy.send_event(UiEvent::PlayerExited { id, result });
                        });
                    }
                    Err(error) => send(&webview, json!({"type":"error", "message":error})),
                },
                Event::UserEvent(UiEvent::PlayerExited { id, result }) => {
                    let message = match result {
                        Ok(status) if status.success() => {
                            "Player closed. Your library is ready.".to_owned()
                        }
                        Ok(status) => {
                            format!("Player exited with {status}. Check the game and try again.")
                        }
                        Err(error) => format!("Could not track player exit: {error}"),
                    };
                    send(
                        &webview,
                        json!({"type":"player_exited", "id":id, "message":message}),
                    );
                }
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => *flow = ControlFlow::Exit,
                _ => {}
            }
        })
    }

    fn launch_options()
    -> Result<(PathBuf, Option<PathBuf>, LogicalSize<f64>, CaptureView), Box<dyn Error>> {
        let args: Vec<_> = env::args_os().collect();
        let default_size = LogicalSize::new(1250.0, 800.0);
        match args.as_slice() {
            [_] => Ok((
                PathBuf::from(env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?)
                    .join("GameStrike")
                    .join("Library"),
                None,
                default_size,
                CaptureView::Library,
            )),
            [_, flag, root] if flag == "--library" => {
                Ok((PathBuf::from(root), None, default_size, CaptureView::Library))
            }
            [_, flag, root, capture_flag, output]
                if flag == "--library" && capture_flag == "--capture-preview" =>
            {
                Ok((PathBuf::from(root), Some(PathBuf::from(output)), default_size, CaptureView::Library))
            }
            [_, flag, root, capture_flag, output, view_flag, view]
                if flag == "--library"
                    && capture_flag == "--capture-preview"
                    && view_flag == "--capture-view" =>
            {
                Ok((PathBuf::from(root), Some(PathBuf::from(output)), default_size, parse_capture_view(view)?))
            }
            [_, flag, root, capture_flag, output, size_flag, width, height]
                if flag == "--library"
                    && capture_flag == "--capture-preview"
                    && size_flag == "--window-size" =>
            {
                let width = width.to_str().ok_or("window width is not UTF-8")?.parse::<u32>()?;
                let height = height.to_str().ok_or("window height is not UTF-8")?.parse::<u32>()?;
                if !(880..=3840).contains(&width) || !(620..=2160).contains(&height) {
                    return Err("capture window size must be between 880x620 and 3840x2160".into());
                }
                Ok((
                    PathBuf::from(root),
                    Some(PathBuf::from(output)),
                    LogicalSize::new(width as f64, height as f64),
                    CaptureView::Library,
                ))
            }
            _ => Err(
                "usage: gamestrike-launcher [--library DIRECTORY [--capture-preview PNG [--capture-view library|long|launch|about | --window-size WIDTH HEIGHT]]]".into(),
            ),
        }
    }

    fn parse_capture_view(raw: &OsString) -> Result<CaptureView, Box<dyn Error>> {
        match raw.to_str() {
            Some("library") => Ok(CaptureView::Library),
            Some("long") => Ok(CaptureView::LongTitle),
            Some("launch") => Ok(CaptureView::Launch),
            Some("about") => Ok(CaptureView::About),
            _ => Err("capture view must be library, long, launch, or about".into()),
        }
    }

    fn capture_preview(
        webview: &wry::WebView,
        path: PathBuf,
        proxy: EventLoopProxy<UiEvent>,
    ) -> Result<(), Box<dyn Error>> {
        // WebView2 captures the rendered library, rather than a staged HTML mock.
        let stream = unsafe { CreateStreamOnHGlobal(Default::default(), true)? };
        let output = stream.clone();
        let handler = CapturePreviewCompletedHandler::create(Box::new(move |completed| {
            let result = (|| -> std::result::Result<(), String> {
                completed.map_err(|error| error.to_string())?;
                unsafe { output.Seek(0, STREAM_SEEK_SET, None) }
                    .map_err(|error| error.to_string())?;
                let mut png = Vec::new();
                let mut chunk = [0u8; 8192];
                loop {
                    let mut count = 0u32;
                    unsafe {
                        output.Read(
                            chunk.as_mut_ptr().cast(),
                            chunk.len() as u32,
                            Some(&mut count),
                        )
                    }
                    .ok()
                    .map_err(|error| error.to_string())?;
                    if count == 0 {
                        break;
                    }
                    png.extend_from_slice(&chunk[..count as usize]);
                    if png.len() > 16 * 1024 * 1024 {
                        return Err("launcher preview exceeds 16 MiB".to_owned());
                    }
                }
                if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
                    return Err("launcher preview is not a PNG".to_owned());
                }
                fs::write(path, png).map_err(|error| error.to_string())
            })();
            let _ = proxy.send_event(UiEvent::CaptureComplete(result));
            Ok(())
        }));
        unsafe {
            webview.webview().CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &handler,
            )?;
        }
        Ok(())
    }

    fn launch(
        library: &Library,
        root: &PathBuf,
        raw_id: &str,
        allow_network: bool,
    ) -> std::result::Result<std::process::Child, String> {
        if !allow_network {
            return Err("Network access must be explicitly accepted for this preview. Strict offline play is unavailable.".into());
        }
        let id = Uuid::parse_str(raw_id).map_err(|_| "Invalid game ID".to_owned())?;
        library
            .player_paths(id)
            .map_err(|error| error.to_string())?;
        let player = env::current_exe()
            .map_err(|error| error.to_string())?
            .with_file_name("gamestrike-player.exe");
        if !player.is_file() {
            return Err(
                "GameStrike player is missing. Build gamestrike-player beside the launcher.".into(),
            );
        }
        Command::new(player)
            .arg("--library")
            .arg(root)
            .arg("--id")
            .arg(id.to_string())
            .arg("--allow-network")
            .spawn()
            .map_err(|error| format!("Could not start player: {error}"))
    }

    fn send_library(webview: &wry::WebView, library: &Library) {
        match library.list() {
            Ok(games) => send(webview, json!({"type":"library", "games":games})),
            Err(error) => send(
                webview,
                json!({"type":"error", "message":format!("Library could not be read: {error}")}),
            ),
        }
    }

    fn send(webview: &wry::WebView, payload: Value) {
        // JSON strings stay quoted inside the JS expression; titles are rendered with textContent.
        let script = format!("window.GameStrike.receive({payload});");
        if let Err(error) = webview.evaluate_script(&script) {
            eprintln!("Could not update GameStrike library: {error}");
        }
    }

    fn is_ui_origin(url: &str) -> bool {
        let Ok(uri) = url.parse::<wry::http::Uri>() else {
            return false;
        };
        (uri.scheme_str() == Some("https")
            && uri
                .authority()
                .is_some_and(|authority| authority.as_str() == "gamestrikeui.localhost"))
            || (uri.scheme_str() == Some("gamestrikeui")
                && uri
                    .authority()
                    .is_some_and(|authority| authority.as_str() == "localhost"))
    }

    fn serve_ui(request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
        let (status, body, mime) = if request.method() != wry::http::Method::GET
            || !is_ui_origin(&request.uri().to_string())
        {
            (StatusCode::FORBIDDEN, &b"denied"[..], "text/plain")
        } else {
            match request.uri().path() {
                "/" | "/index.html" => (
                    StatusCode::OK,
                    &include_bytes!("../ui/index.html")[..],
                    "text/html; charset=utf-8",
                ),
                "/app.js" => (
                    StatusCode::OK,
                    &include_bytes!("../ui/app.js")[..],
                    "text/javascript; charset=utf-8",
                ),
                "/styles.css" => (
                    StatusCode::OK,
                    &include_bytes!("../ui/styles.css")[..],
                    "text/css; charset=utf-8",
                ),
                "/fox.png" => (
                    StatusCode::OK,
                    &include_bytes!("../assets/branding/gamestrike-fox-icon.png")[..],
                    "image/png",
                ),
                "/pwds.svg" => (
                    StatusCode::OK,
                    &include_bytes!("../assets/branding/PWDS-logo-orange.svg")[..],
                    "image/svg+xml",
                ),
                _ => (StatusCode::NOT_FOUND, &b"not found"[..], "text/plain"),
            }
        };
        let mut response = Response::new(Cow::Borrowed(body));
        *response.status_mut() = status;
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, header::HeaderValue::from_static(mime));
        response.headers_mut().insert(header::HeaderName::from_static("content-security-policy"), header::HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; base-uri 'none'; frame-src 'none'; form-action 'none'; connect-src 'none'"));
        response.headers_mut().insert(
            header::HeaderName::from_static("x-content-type-options"),
            header::HeaderValue::from_static("nosniff"),
        );
        response
    }

    fn pick_game_file(owner: isize) -> std::result::Result<Option<PathBuf>, String> {
        let mut selected = vec![0u16; 32_768];
        let filter: Vec<u16> = "HTML games (*.html;*.htm;*.zip)\0*.html;*.htm;*.zip\0HTML files (*.html;*.htm)\0*.html;*.htm\0ZIP archives (*.zip)\0*.zip\0\0".encode_utf16().collect();
        let title: Vec<u16> = "Add a local HTML game\0".encode_utf16().collect();
        let mut dialog = OPENFILENAMEW {
            lStructSize: size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: owner as *mut _,
            lpstrFilter: filter.as_ptr(),
            lpstrFile: selected.as_mut_ptr(),
            nMaxFile: selected.len() as u32,
            lpstrTitle: title.as_ptr(),
            Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
            ..Default::default()
        };
        // The dialog supplies only a selected path; Library::import validates the file and ZIP.
        if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
            let code = unsafe { CommDlgExtendedError() };
            return if code == 0 {
                Ok(None)
            } else {
                Err(format!("File picker failed (Windows error {code:#x})."))
            };
        }
        let end = selected
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(selected.len());
        Ok(Some(PathBuf::from(OsString::from_wide(&selected[..end]))))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn ui_protocol_serves_only_bundled_paths_on_its_exact_origin() {
            let allowed = Request::builder()
                .uri("https://gamestrikeui.localhost/index.html")
                .body(Vec::new())
                .unwrap();
            let response = serve_ui(allowed);
            assert_eq!(response.status(), StatusCode::OK);
            assert!(response.body().as_ref().starts_with(b"<!doctype html>"));

            for url in [
                "https://other.localhost/index.html",
                "https://gamestrikeui.localhost.evil/index.html",
                "https://gamestrikeui.localhost/private.txt",
            ] {
                let request = Request::builder().uri(url).body(Vec::new()).unwrap();
                assert_ne!(serve_ui(request).status(), StatusCode::OK, "{url}");
            }
        }
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows_launcher::run()
}
