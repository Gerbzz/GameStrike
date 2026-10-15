#![deny(unsafe_code)]

#[cfg(not(windows))]
fn main() {
    eprintln!("The current GameStrike player prototype is Windows-only.");
}

#[cfg(windows)]
mod windows_player {
    use gamestrike::{Library, safe_content_path};
    use percent_encoding::percent_decode_str;
    use std::{
        borrow::Cow,
        env,
        error::Error,
        fs,
        io::{self, Read},
        path::{Path, PathBuf},
        sync::mpsc,
        time::{Duration, Instant},
    };
    use tao::{
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoop},
        window::WindowBuilder,
    };
    use uuid::Uuid;
    use webview2_com::{
        LaunchingExternalUriSchemeEventHandler, Microsoft::Web::WebView2::Win32::ICoreWebView2_18,
        NavigationStartingEventHandler, take_pwstr,
    };
    use windows::core::{Interface, PWSTR};
    use wry::{
        NewWindowResponse, PermissionResponse, WebContext, WebViewBuilder,
        WebViewBuilderExtWindows, WebViewExtWindows,
        http::{
            Request, Response, StatusCode,
            header::{CONTENT_TYPE, HeaderValue},
        },
    };

    const MAX_ASSET_BYTES: u64 = 128 * 1024 * 1024;
    const MEDIUM_INTEGRITY_RID: u32 = 0x2000;

    // Only this small module calls native Windows token-query APIs. No token is
    // changed or duplicated; every uncertain result becomes a launch error.
    #[allow(unsafe_code)]
    mod integrity {
        use std::{io, mem::size_of, ptr};
        use windows_sys::Win32::{
            Foundation::{CloseHandle, HANDLE},
            Security::{
                GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, IsValidSid,
                TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel,
            },
            System::Threading::{GetCurrentProcess, OpenProcessToken},
        };

        struct Token(HANDLE);

        impl Drop for Token {
            fn drop(&mut self) {
                unsafe { CloseHandle(self.0) };
            }
        }

        pub(super) fn current_integrity_rid() -> io::Result<u32> {
            let mut raw_token = ptr::null_mut();
            if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw_token) } == 0 {
                return Err(io::Error::last_os_error());
            }
            if raw_token.is_null() {
                return Err(io::Error::other("process token handle is missing"));
            }
            let token = Token(raw_token);
            let mut required = 0u32;
            unsafe {
                GetTokenInformation(
                    token.0,
                    TokenIntegrityLevel,
                    ptr::null_mut(),
                    0,
                    &mut required,
                )
            };
            if required < size_of::<TOKEN_MANDATORY_LABEL>() as u32 || required > 4096 {
                return Err(io::Error::other("integrity token size is invalid"));
            }
            let word = size_of::<usize>();
            let words = (required as usize).div_ceil(word);
            let mut buffer = vec![0usize; words];
            let mut returned = 0u32;
            if unsafe {
                GetTokenInformation(
                    token.0,
                    TokenIntegrityLevel,
                    buffer.as_mut_ptr().cast(),
                    required,
                    &mut returned,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            if returned < size_of::<TOKEN_MANDATORY_LABEL>() as u32 || returned > required {
                return Err(io::Error::other("integrity token response is invalid"));
            }
            let label = unsafe { &*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
            let sid = label.Label.Sid;
            if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
                return Err(io::Error::other("integrity SID is invalid"));
            }
            let count_ptr = unsafe { GetSidSubAuthorityCount(sid) };
            if count_ptr.is_null() {
                return Err(io::Error::other("integrity SID has no authority count"));
            }
            let count = unsafe { *count_ptr };
            if count == 0 || count > 15 {
                return Err(io::Error::other("integrity SID authority count is invalid"));
            }
            let rid_ptr = unsafe { GetSidSubAuthority(sid, u32::from(count - 1)) };
            if rid_ptr.is_null() {
                return Err(io::Error::other("integrity SID authority is missing"));
            }
            Ok(unsafe { *rid_ptr })
        }
    }

    pub fn run() -> Result<(), Box<dyn Error>> {
        let args: Vec<_> = env::args_os().collect();
        if args.len() != 6
            || args.get(1).and_then(|arg| arg.to_str()) != Some("--library")
            || args.get(3).and_then(|arg| arg.to_str()) != Some("--id")
        {
            return Err(player_usage().into());
        }
        let boundary_smoke = args[5].to_str() == Some("--boundary-smoke");
        let smoke = match args[5].to_str() {
            Some("--allow-network") => false,
            Some("--smoke" | "--boundary-smoke") => true,
            _ => return Err(player_usage().into()),
        };
        ensure_standard_integrity()?;
        let library = Library::open(PathBuf::from(&args[2]))?;
        let id = Uuid::parse_str(args[4].to_str().ok_or("game ID is not UTF-8")?)?;
        let game = library
            .list()?
            .into_iter()
            .find(|game| game.id == id)
            .ok_or("game ID is not in this library")?;
        let (content, profile) = library.player_paths(id)?;
        if smoke {
            verify_smoke_fixture(&content, boundary_smoke)?;
        }

        // Wry maps this custom scheme to HTTPS on Windows. The scheme and
        // persistent browser data folder are unique and stable for this game.
        let scheme = format!("gamestrike{}", id.simple());
        let host = format!("{scheme}.localhost");
        let entry_url = format!("https://{host}/index.html");
        let navigation_scheme = scheme.clone();
        let navigation_host = host.clone();
        let guard_origin = (scheme.clone(), host.clone());
        let asset_scheme = scheme.clone();
        let asset_host = host.clone();

        let event_loop = EventLoop::new();
        let window = WindowBuilder::new()
            .with_title(format!("GameStrike | {} | by PWDS", game.title))
            .build(&event_loop)?;
        let mut context = WebContext::new(Some(profile));
        let (title_sender, title_receiver) = mpsc::channel::<String>();
        let mut builder = WebViewBuilder::new_with_web_context(&mut context)
            .with_custom_protocol(scheme, move |_, request| {
                serve_asset(&content, &asset_scheme, &asset_host, request)
            })
            .with_https_scheme(true)
            .with_navigation_handler(move |url| {
                if url == "about:blank" {
                    return true;
                }
                is_game_origin(&url, &navigation_scheme, &navigation_host)
            })
            .with_new_window_req_handler(|_, _| {
                eprintln!("GameStrike denied new window");
                NewWindowResponse::Deny
            })
            .with_download_started_handler(|_, _| {
                eprintln!("GameStrike denied download");
                false
            })
            .with_permission_handler(|_| {
                eprintln!("GameStrike denied permission");
                PermissionResponse::Deny
            })
            .with_drag_drop_handler(|_| true)
            .with_browser_accelerator_keys(false)
            .with_default_context_menus(false)
            .with_devtools(false);
        if smoke {
            builder = builder.with_document_title_changed_handler(move |title| {
                if fixture_reported(&title, boundary_smoke) {
                    let _ = title_sender.send(title);
                }
            });
        }
        let webview = builder.build(&window)?;
        disable_privileged_webview_features(&webview)?;
        install_native_navigation_guards(&webview, &guard_origin)?;

        // Configure handlers before navigating to imported JavaScript.
        webview.load_url(&entry_url)?;
        eprintln!(
            "This game's web content can request network resources; launch only games you trust with that access."
        );
        let started = Instant::now();
        let mut observed_at = None;
        event_loop.run(move |event, _, control_flow| {
            let _keep_browser_state = (&context, &webview);
            *control_flow = if smoke {
                ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100))
            } else {
                ControlFlow::Wait
            };
            if smoke {
                if observed_at.is_none() {
                    if let Ok(title) = title_receiver.try_recv() {
                        println!("{title}");
                        observed_at = Some(Instant::now());
                    }
                }
                if observed_at.is_some_and(|at: Instant| at.elapsed() >= Duration::from_secs(1)) {
                    *control_flow = ControlFlow::Exit;
                }
                if started.elapsed() >= Duration::from_secs(15) {
                    eprintln!("Benign fixture did not report completion.");
                    std::process::exit(2);
                }
            }
            if let Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } = event
            {
                *control_flow = ControlFlow::Exit;
            }
        });
    }

    fn player_usage() -> &'static str {
        "usage: gamestrike-player --library DIRECTORY --id GAME_UUID --allow-network\nStrict offline play is not available yet; imported games require explicit network opt-in."
    }

    fn fixture_reported(title: &str, boundary: bool) -> bool {
        if boundary {
            title.starts_with("GameStrike boundary fixture launch=")
        } else {
            title.starts_with("GameStrike fixture launch ")
        }
    }

    fn require_medium_integrity(rid: Option<u32>) -> Result<(), &'static str> {
        match rid {
            Some(MEDIUM_INTEGRITY_RID) => Ok(()),
            Some(_) => Err("GameStrike player requires standard Windows integrity"),
            None => Err("GameStrike player could not verify Windows integrity"),
        }
    }

    fn ensure_standard_integrity() -> Result<(), Box<dyn Error>> {
        ensure_integrity_with(integrity::current_integrity_rid)
    }

    fn ensure_integrity_with(
        probe: impl FnOnce() -> io::Result<u32>,
    ) -> Result<(), Box<dyn Error>> {
        let rid = probe()?;
        require_medium_integrity(Some(rid)).map_err(Into::into)
    }

    #[allow(unsafe_code)]
    fn disable_privileged_webview_features(webview: &wry::WebView) -> Result<(), Box<dyn Error>> {
        // No game document has loaded yet. Fail the launch if the runtime cannot
        // explicitly disable host objects and messages.
        let settings = unsafe { webview.webview().Settings()? };
        unsafe {
            settings.SetAreHostObjectsAllowed(false)?;
            settings.SetIsWebMessageEnabled(false)?;
            settings.SetAreDefaultScriptDialogsEnabled(false)?;
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    fn install_native_navigation_guards(
        webview: &wry::WebView,
        origin: &(String, String),
    ) -> Result<(), Box<dyn Error>> {
        let webview = webview.webview();
        let (scheme, host) = origin.clone();
        let mut token = 0;
        unsafe {
            webview.add_FrameNavigationStarting(
                &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    args.SetCancel(true)?;
                    let mut raw = PWSTR::null();
                    args.Uri(&mut raw)?;
                    let uri = take_pwstr(raw);
                    let allow = uri == "about:blank" || is_game_origin(&uri, &scheme, &host);
                    if !allow {
                        eprintln!("GameStrike denied frame navigation: {uri}");
                    }
                    args.SetCancel(!allow)?;
                    Ok(())
                })),
                &mut token,
            )?;
        }

        // A registered OS URI handler can be invoked even for a subframe.
        // Require this WebView2 interface and cancel before any game loads.
        let external: ICoreWebView2_18 = webview.cast()?;
        unsafe {
            external.add_LaunchingExternalUriScheme(
                &LaunchingExternalUriSchemeEventHandler::create(Box::new(|_, args| {
                    if let Some(args) = args {
                        args.SetCancel(true)?;
                        eprintln!("GameStrike denied external URI launch");
                    }
                    Ok(())
                })),
                &mut token,
            )?;
        }
        Ok(())
    }

    fn verify_smoke_fixture(content: &Path, boundary: bool) -> Result<(), Box<dyn Error>> {
        let mut entries = fs::read_dir(content)?;
        let only_entry = entries.next().ok_or("smoke fixture is missing")??;
        if only_entry.file_name().to_str() != Some("index.html")
            || !only_entry.file_type()?.is_file()
            || entries.next().is_some()
            || fs::read(only_entry.path())?.as_slice()
                != if boundary {
                    include_bytes!("../fixtures/boundary-smoke.html").as_slice()
                } else {
                    include_bytes!("../fixtures/save-counter.html").as_slice()
                }
        {
            return Err("--smoke only runs the exact bundled benign fixture".into());
        }
        Ok(())
    }

    fn is_game_origin(url: &str, scheme: &str, host: &str) -> bool {
        let Ok(uri) = url.parse::<wry::http::Uri>() else {
            return false;
        };
        let authority = uri.authority().map(|value| value.as_str());
        (uri.scheme_str() == Some("https") && authority == Some(host))
            || (uri.scheme_str() == Some(scheme) && authority == Some("localhost"))
    }

    fn serve_asset(
        root: &Path,
        scheme: &str,
        host: &str,
        request: Request<Vec<u8>>,
    ) -> Response<Cow<'static, [u8]>> {
        if !is_game_origin(&request.uri().to_string(), scheme, host) {
            return reply(
                StatusCode::FORBIDDEN,
                b"origin denied".to_vec(),
                "text/plain",
            );
        }
        if request.method() != wry::http::Method::GET {
            return reply(
                StatusCode::METHOD_NOT_ALLOWED,
                b"method not allowed".to_vec(),
                "text/plain",
            );
        }
        let raw = request.uri().path().trim_start_matches('/');
        let raw = if raw.is_empty() { "index.html" } else { raw };
        let Ok(decoded) = percent_decode_str(raw).decode_utf8() else {
            return reply(
                StatusCode::BAD_REQUEST,
                b"invalid path encoding".to_vec(),
                "text/plain",
            );
        };
        let Ok(relative) = safe_content_path(&decoded) else {
            return reply(
                StatusCode::BAD_REQUEST,
                b"invalid path".to_vec(),
                "text/plain",
            );
        };
        let Ok(target) = fs::canonicalize(root.join(&relative)) else {
            return reply(
                StatusCode::NOT_FOUND,
                b"asset not found".to_vec(),
                "text/plain",
            );
        };
        if !target.starts_with(root) || !target.is_file() {
            return reply(
                StatusCode::FORBIDDEN,
                b"asset denied".to_vec(),
                "text/plain",
            );
        }
        let Ok(file) = fs::File::open(&target) else {
            return reply(
                StatusCode::NOT_FOUND,
                b"asset not found".to_vec(),
                "text/plain",
            );
        };
        let mut bytes = Vec::new();
        if file
            .take(MAX_ASSET_BYTES + 1)
            .read_to_end(&mut bytes)
            .is_err()
        {
            return reply(
                StatusCode::INTERNAL_SERVER_ERROR,
                b"asset read failed".to_vec(),
                "text/plain",
            );
        }
        if bytes.len() as u64 > MAX_ASSET_BYTES {
            return reply(
                StatusCode::PAYLOAD_TOO_LARGE,
                b"asset too large".to_vec(),
                "text/plain",
            );
        }
        reply(StatusCode::OK, bytes, mime(&relative))
    }

    fn reply(
        status: StatusCode,
        body: Vec<u8>,
        mime: &'static str,
    ) -> Response<Cow<'static, [u8]>> {
        let mut response = Response::new(Cow::Owned(body));
        *response.status_mut() = status;
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static(mime));
        response
    }

    fn mime(path: &Path) -> &'static str {
        match path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "html" | "htm" => "text/html; charset=utf-8",
            "js" | "mjs" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "json" => "application/json",
            "wasm" => "application/wasm",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "svg" => "image/svg+xml",
            "webp" => "image/webp",
            "mp3" => "audio/mpeg",
            "ogg" => "audio/ogg",
            "wav" => "audio/wav",
            "mp4" => "video/mp4",
            "webm" => "video/webm",
            "woff" => "font/woff",
            "woff2" => "font/woff2",
            "ttf" => "font/ttf",
            "otf" => "font/otf",
            _ => "application/octet-stream",
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn boundary_probe_waits_for_results_instead_of_initial_title() {
            assert!(!fixture_reported(
                "GameStrike boundary fixture starting",
                true
            ));
            assert!(fixture_reported(
                "GameStrike boundary fixture launch=2;popup=null",
                true
            ));
            assert!(fixture_reported("GameStrike fixture launch 3", false));
            assert!(!fixture_reported("GameStrike save fixture", false));
        }

        #[test]
        fn asset_handler_serves_only_a_game_content_root() {
            let dir = tempfile::tempdir().unwrap();
            let content = dir.path().join("content");
            fs::create_dir(&content).unwrap();
            fs::write(content.join("index.html"), b"<title>fixture</title>").unwrap();
            fs::write(dir.path().join("secret.txt"), b"private").unwrap();
            let content = fs::canonicalize(content).unwrap();
            let request = Request::builder()
                .uri("https://game.localhost/index.html")
                .body(Vec::new())
                .unwrap();
            let response = serve_asset(&content, "game", "game.localhost", request);
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.body().as_ref(), b"<title>fixture</title>");

            for path in ["/%2e%2e/secret.txt", "/C:%2fsecret.txt", "/CON.txt"] {
                let request = Request::builder()
                    .uri(format!("https://game.localhost{path}"))
                    .body(Vec::new())
                    .unwrap();
                let response = serve_asset(&content, "game", "game.localhost", request);
                assert_ne!(response.status(), StatusCode::OK);
                assert!(
                    !response
                        .body()
                        .as_ref()
                        .windows(7)
                        .any(|bytes| bytes == b"private")
                );
            }
        }

        #[test]
        fn navigation_and_asset_origins_are_exact() {
            assert!(is_game_origin(
                "https://game.localhost/index.html",
                "game",
                "game.localhost"
            ));
            for url in [
                "https://other.localhost/index.html",
                "https://game.localhost.evil/index.html",
                "https://user@game.localhost/index.html",
                "file:///C:/fixture.txt",
                "javascript:alert(1)",
                "ms-appx:///fixture.txt",
            ] {
                assert!(!is_game_origin(url, "game", "game.localhost"), "{url}");
            }
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("index.html"), b"fixture").unwrap();
            let request = Request::builder()
                .uri("https://other.localhost/index.html")
                .body(Vec::new())
                .unwrap();
            assert_eq!(
                serve_asset(dir.path(), "game", "game.localhost", request).status(),
                StatusCode::FORBIDDEN
            );
        }

        #[test]
        fn integrity_policy_refuses_elevated_and_unknown_levels() {
            assert!(require_medium_integrity(Some(MEDIUM_INTEGRITY_RID)).is_ok());
            for rid in [0, 0x1000, 0x2100, 0x3000, 0x4000, 0x5000, u32::MAX] {
                assert!(require_medium_integrity(Some(rid)).is_err(), "{rid:#x}");
            }
            assert!(require_medium_integrity(None).is_err());
            let query_error =
                ensure_integrity_with(|| Err(io::Error::other("token query failed"))).unwrap_err();
            assert!(query_error.to_string().contains("token query failed"));
        }

        #[test]
        fn current_windows_token_is_queried_before_launch() {
            let rid = integrity::current_integrity_rid().expect("token integrity query failed");
            eprintln!("GameStrike test process integrity RID: {rid:#x}");
            assert_eq!(
                ensure_standard_integrity().is_ok(),
                rid == MEDIUM_INTEGRITY_RID
            );
        }

        #[test]
        fn smoke_mode_accepts_only_the_exact_bundled_fixture() {
            let dir = tempfile::tempdir().unwrap();
            let entry = dir.path().join("index.html");
            fs::write(&entry, include_bytes!("../fixtures/save-counter.html")).unwrap();
            assert!(verify_smoke_fixture(dir.path(), false).is_ok());
            assert!(verify_smoke_fixture(dir.path(), true).is_err());
            fs::write(&entry, b"<html>different fixture</html>").unwrap();
            assert!(verify_smoke_fixture(dir.path(), false).is_err());
            fs::write(&entry, include_bytes!("../fixtures/save-counter.html")).unwrap();
            fs::write(dir.path().join("extra.js"), b"// extra file").unwrap();
            assert!(verify_smoke_fixture(dir.path(), false).is_err());
        }
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows_player::run()
}
