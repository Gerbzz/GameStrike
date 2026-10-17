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
        io::{self, Read, Write},
        net::{Ipv4Addr, TcpListener},
        path::{Path, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };
    use tao::{
        dpi::LogicalSize,
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
            header::{CONTENT_TYPE, HeaderName, HeaderValue},
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

    /// How the player was asked to run. Fixture modes load only exact bundled first-party HTML.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Mode {
        Offline,
        Network,
        Fixture(Fixture),
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Fixture {
        Save,
        Boundary,
        ProbeOffline,
        ProbeNetwork,
    }

    impl Mode {
        fn parse(flag: Option<&str>) -> Option<Self> {
            Some(match flag? {
                "--offline" => Mode::Offline,
                "--allow-network" => Mode::Network,
                "--smoke" => Mode::Fixture(Fixture::Save),
                "--boundary-smoke" => Mode::Fixture(Fixture::Boundary),
                "--probe-offline" => Mode::Fixture(Fixture::ProbeOffline),
                "--probe-network" => Mode::Fixture(Fixture::ProbeNetwork),
                _ => return None,
            })
        }

        fn offline(self) -> bool {
            !matches!(
                self,
                Mode::Network | Mode::Fixture(Fixture::Boundary | Fixture::ProbeNetwork)
            )
        }

        fn fixture(self) -> Option<Fixture> {
            match self {
                Mode::Fixture(fixture) => Some(fixture),
                _ => None,
            }
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
        let mode = Mode::parse(args[5].to_str()).ok_or_else(player_usage)?;
        let fixture = mode.fixture();
        // Imported content needs a standard-integrity token. Fixture modes run only exact
        // bundled first-party bytes, so CI runners with elevated tokens can still exercise them.
        if fixture.is_none() {
            ensure_standard_integrity()?;
        }
        let library = Library::open(PathBuf::from(&args[2]))?;
        let id = Uuid::parse_str(args[4].to_str().ok_or("game ID is not UTF-8")?)?;
        let game = library
            .list()?
            .into_iter()
            .find(|game| game.id == id)
            .ok_or("game ID is not in this library")?;
        let (content, profile) = library.player_paths(id)?;
        let _lock = library.lock_game(id)?;
        if let Some(fixture) = fixture {
            verify_smoke_fixture(&content, fixture)?;
        }
        let blocked = Arc::new(AtomicUsize::new(0));
        let browser_args = if mode.offline() {
            let port = start_sinkhole_proxy(Arc::clone(&blocked))?;
            offline_browser_args(port)
        } else {
            BASE_BROWSER_ARGS.to_owned()
        };
        let csp = if mode.offline() {
            OFFLINE_CSP
        } else {
            NETWORK_CSP
        };

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
            .with_title(format!(
                "{} · GameStrike{}",
                game.title,
                if mode.offline() {
                    " · Offline"
                } else {
                    " · Network allowed"
                }
            ))
            .with_inner_size(LogicalSize::new(1280.0, 800.0))
            .with_min_inner_size(LogicalSize::new(480.0, 360.0))
            .build(&event_loop)?;
        let mut context = WebContext::new(Some(profile));
        let (title_sender, title_receiver) = mpsc::channel::<String>();
        let mut builder = WebViewBuilder::new_with_web_context(&mut context)
            .with_custom_protocol(scheme, move |_, request| {
                serve_asset(&content, &asset_scheme, &asset_host, csp, request)
            })
            .with_additional_browser_args(browser_args)
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
        if let Some(fixture) = fixture {
            builder = builder.with_document_title_changed_handler(move |title| {
                if fixture_reported(&title, fixture) {
                    let _ = title_sender.send(title);
                }
            });
        }
        let webview = builder.build(&window)?;
        disable_privileged_webview_features(&webview)?;
        install_native_navigation_guards(&webview, &guard_origin)?;

        // Configure handlers before navigating to imported JavaScript.
        webview.load_url(&entry_url)?;
        if !mode.offline() {
            eprintln!(
                "This game's web content can request network resources; launch only games you trust with that access."
            );
        }
        let started = Instant::now();
        let mut observed_at = None;
        event_loop.run(move |event, _, control_flow| {
            let _keep_browser_state = (&context, &webview, &_lock);
            *control_flow = if fixture.is_some() {
                ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100))
            } else {
                ControlFlow::Wait
            };
            let mut exit = false;
            if fixture.is_some() {
                if observed_at.is_none()
                    && let Ok(title) = title_receiver.try_recv()
                {
                    println!("{title}");
                    observed_at = Some(Instant::now());
                }
                exit =
                    observed_at.is_some_and(|at: Instant| at.elapsed() >= Duration::from_secs(1));
                if started.elapsed() >= Duration::from_secs(30) {
                    eprintln!("Benign fixture did not report completion.");
                    std::process::exit(2);
                }
            }
            if let Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } = event
            {
                exit = true;
            }
            if exit {
                // The launcher reads this line to tell the player what offline mode stopped.
                println!("{BLOCKED_PREFIX}{}", blocked.load(Ordering::SeqCst));
                let _ = io::stdout().flush();
                *control_flow = ControlFlow::Exit;
            }
        });
    }

    const BLOCKED_PREFIX: &str = "gamestrike-network-blocked=";
    const BASE_BROWSER_ARGS: &str =
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";
    /// Offline pages may use their own origin plus inline, data:, and blob: content.
    const OFFLINE_CSP: &str = "default-src 'self' data: blob: 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval'; object-src 'none'; base-uri 'self'; form-action 'self'";
    /// Network play still blocks plugins and base-URL hijacking.
    const NETWORK_CSP: &str = "object-src 'none'; base-uri 'self'";

    /// All HTTP(S) and WebSocket traffic is sent to a proxy this process owns, including
    /// loopback, and WebRTC may not use UDP outside that proxy. The CSP blocks the same
    /// requests earlier; the proxy covers paths CSP does not, such as WebRTC.
    fn offline_browser_args(port: u16) -> String {
        format!(
            "{BASE_BROWSER_ARGS} --proxy-server=http://127.0.0.1:{port} --proxy-bypass-list=<-loopback> --force-webrtc-ip-handling-policy=disable_non_proxied_udp"
        )
    }

    /// Accepts each proxy connection, counts it, and answers 403 without contacting anything.
    fn start_sinkhole_proxy(blocked: Arc<AtomicUsize>) -> io::Result<u16> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                blocked.fetch_add(1, Ordering::SeqCst);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut head = [0u8; 512];
                let count = stream.read(&mut head).unwrap_or(0);
                let line = String::from_utf8_lossy(&head[..count]);
                let target = line
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("unknown");
                eprintln!("GameStrike offline mode blocked a request to {target}");
                let _ = stream.write_all(
                    b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        Ok(port)
    }

    fn player_usage() -> &'static str {
        "usage: gamestrike-player --library DIRECTORY --id GAME_UUID --offline|--allow-network\n--offline blocks network requests; --allow-network lets the game contact local and remote services."
    }

    fn fixture_reported(title: &str, fixture: Fixture) -> bool {
        match fixture {
            Fixture::Boundary => title.starts_with("GameStrike boundary fixture launch="),
            Fixture::Save => title.starts_with("GameStrike fixture launch "),
            Fixture::ProbeOffline | Fixture::ProbeNetwork => {
                title.starts_with("GameStrike isolation probe launch=")
            }
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

    fn verify_smoke_fixture(content: &Path, fixture: Fixture) -> Result<(), Box<dyn Error>> {
        let expected = match fixture {
            Fixture::Save => include_bytes!("../fixtures/save-counter.html").as_slice(),
            Fixture::Boundary => include_bytes!("../fixtures/boundary-smoke.html").as_slice(),
            Fixture::ProbeOffline | Fixture::ProbeNetwork => {
                include_bytes!("../fixtures/isolation-probe.html").as_slice()
            }
        };
        let mut entries = fs::read_dir(content)?;
        let only_entry = entries.next().ok_or("smoke fixture is missing")??;
        if only_entry.file_name().to_str() != Some("index.html")
            || !only_entry.file_type()?.is_file()
            || entries.next().is_some()
            || fs::read(only_entry.path())?.as_slice() != expected
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
        csp: &'static str,
        request: Request<Vec<u8>>,
    ) -> Response<Cow<'static, [u8]>> {
        let mut response = serve_asset_body(root, scheme, host, request);
        let headers = response.headers_mut();
        headers.insert(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static(csp),
        );
        headers.insert(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        );
        headers.insert(
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("no-referrer"),
        );
        response
    }

    fn serve_asset_body(
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
                Fixture::Boundary
            ));
            assert!(fixture_reported(
                "GameStrike boundary fixture launch=2;popup=null",
                Fixture::Boundary
            ));
            assert!(fixture_reported(
                "GameStrike fixture launch 3",
                Fixture::Save
            ));
            assert!(!fixture_reported("GameStrike save fixture", Fixture::Save));
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
            let response = serve_asset(&content, "game", "game.localhost", OFFLINE_CSP, request);
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.body().as_ref(), b"<title>fixture</title>");

            for path in ["/%2e%2e/secret.txt", "/C:%2fsecret.txt", "/CON.txt"] {
                let request = Request::builder()
                    .uri(format!("https://game.localhost{path}"))
                    .body(Vec::new())
                    .unwrap();
                let response =
                    serve_asset(&content, "game", "game.localhost", OFFLINE_CSP, request);
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
                serve_asset(dir.path(), "game", "game.localhost", OFFLINE_CSP, request).status(),
                StatusCode::FORBIDDEN
            );
        }

        #[test]
        fn network_needs_an_explicit_flag_and_unknown_modes_fail() {
            assert_eq!(Mode::parse(Some("--offline")), Some(Mode::Offline));
            assert!(Mode::Offline.offline());
            assert!(!Mode::parse(Some("--allow-network")).unwrap().offline());
            assert!(Mode::parse(Some("--probe-offline")).unwrap().offline());
            assert!(!Mode::parse(Some("--probe-network")).unwrap().offline());
            assert!(Mode::parse(Some("--smoke")).unwrap().offline());
            for flag in [None, Some(""), Some("--network"), Some("--OFFLINE")] {
                assert_eq!(Mode::parse(flag), None, "{flag:?}");
            }
            assert_eq!(Mode::Offline.fixture(), None);
            assert_eq!(Mode::Network.fixture(), None);
        }

        #[test]
        fn offline_mode_routes_everything_through_the_owned_proxy() {
            let args = offline_browser_args(4321);
            assert!(args.contains("--proxy-server=http://127.0.0.1:4321"));
            assert!(args.contains("--proxy-bypass-list=<-loopback>"));
            assert!(args.contains("--force-webrtc-ip-handling-policy=disable_non_proxied_udp"));
            assert!(args.starts_with(BASE_BROWSER_ARGS));
            for directive in ["default-src 'self' data: blob:", "object-src 'none'"] {
                assert!(OFFLINE_CSP.contains(directive), "{directive}");
            }
            assert!(!OFFLINE_CSP.contains("http:") && !OFFLINE_CSP.contains('*'));
        }

        #[test]
        fn sinkhole_proxy_answers_403_and_counts_attempts() {
            let blocked = Arc::new(AtomicUsize::new(0));
            let port = start_sinkhole_proxy(Arc::clone(&blocked)).unwrap();
            let mut stream = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            stream
                .write_all(b"CONNECT example.invalid:443 HTTP/1.1\r\nHost: example.invalid\r\n\r\n")
                .unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).unwrap();
            assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
            assert_eq!(blocked.load(Ordering::SeqCst), 1);
        }

        #[test]
        fn every_asset_response_carries_the_selected_csp() {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("index.html"), b"fixture").unwrap();
            let content = fs::canonicalize(dir.path()).unwrap();
            for (url, csp) in [
                ("https://game.localhost/index.html", OFFLINE_CSP),
                ("https://game.localhost/missing.js", OFFLINE_CSP),
                ("https://other.localhost/index.html", NETWORK_CSP),
            ] {
                let request = Request::builder().uri(url).body(Vec::new()).unwrap();
                let response = serve_asset(&content, "game", "game.localhost", csp, request);
                assert_eq!(
                    response.headers().get("content-security-policy").unwrap(),
                    csp,
                    "{url}"
                );
                assert_eq!(
                    response.headers().get("x-content-type-options").unwrap(),
                    "nosniff"
                );
            }
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
            assert!(verify_smoke_fixture(dir.path(), Fixture::Save).is_ok());
            assert!(verify_smoke_fixture(dir.path(), Fixture::Boundary).is_err());
            fs::write(&entry, b"<html>different fixture</html>").unwrap();
            assert!(verify_smoke_fixture(dir.path(), Fixture::Save).is_err());
            fs::write(&entry, include_bytes!("../fixtures/save-counter.html")).unwrap();
            fs::write(dir.path().join("extra.js"), b"// extra file").unwrap();
            assert!(verify_smoke_fixture(dir.path(), Fixture::Save).is_err());
        }
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows_player::run()
}
