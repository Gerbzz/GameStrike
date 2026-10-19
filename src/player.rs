#![deny(unsafe_code)]
//! Runs one imported game in its own window, origin, and browser data store.
//!
//! - Windows: WebView2, origin `https://gamestrike<uuid>.localhost`, user-data folder `LIBRARY/profiles/<uuid>`.
//! - Linux: WebKitGTK, origin `gamestrike<uuid>://localhost`, data and cache directories `LIBRARY/profiles/<uuid>`.
//! - macOS 14+: WKWebView, origin `gamestrike<uuid>://localhost`, `WKWebsiteDataStore` with identifier `<uuid>`.

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn main() {
    eprintln!("The GameStrike player supports Windows, Linux, and macOS only.");
    std::process::exit(1);
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
mod game_player {
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
        window::{Window, WindowBuilder},
    };
    use uuid::Uuid;
    use wry::{
        NewWindowResponse, PermissionResponse, WebContext, WebViewBuilder,
        http::{
            Request, Response, StatusCode,
            header::{CONTENT_TYPE, HeaderName, HeaderValue},
        },
    };

    const MAX_ASSET_BYTES: u64 = 128 * 1024 * 1024;

    // Only this small module calls native Windows token-query APIs. No token is
    // changed or duplicated; every uncertain result becomes a launch error.
    #[cfg(windows)]
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

        pub(super) const MEDIUM_INTEGRITY_RID: u32 = 0x2000;

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

    // The Unix counterpart of the integrity check: read-only queries of this process's IDs.
    #[cfg(unix)]
    #[allow(unsafe_code)]
    mod privileges {
        /// Real and effective user and group IDs of this process.
        pub(super) fn ids() -> [u32; 4] {
            // These libc calls take no arguments, cannot fail, and change nothing.
            unsafe {
                [
                    libc::getuid(),
                    libc::geteuid(),
                    libc::getgid(),
                    libc::getegid(),
                ]
            }
        }
    }

    /// How the player was asked to run. Fixture modes load only exact bundled first-party HTML.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Mode {
        Offline,
        Network,
        Fixture(Fixture),
        /// Deletes the game's engine data that lives outside the library (macOS only), then exits.
        ClearEngineData,
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
                "--clear-engine-data" => Mode::ClearEngineData,
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

    /// The per-game origin. Each game gets its own custom scheme, so origins never overlap.
    #[derive(Clone, Debug)]
    struct GameOrigin {
        scheme: String,
        host: String,
    }

    impl GameOrigin {
        fn new(id: Uuid) -> Self {
            let scheme = format!("gamestrike{}", id.simple());
            let host = format!("{scheme}.localhost");
            Self { scheme, host }
        }

        /// Wry maps the custom scheme to `https://<scheme>.localhost` on Windows only.
        fn entry_url(&self) -> String {
            if cfg!(windows) {
                format!("https://{}/index.html", self.host)
            } else {
                format!("{}://localhost/index.html", self.scheme)
            }
        }

        fn allows(&self, url: &str) -> bool {
            is_game_origin(url, &self.scheme, &self.host)
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
        // Imported content needs a standard, unprivileged identity. Fixture modes run only exact
        // bundled first-party bytes, so CI runners with elevated tokens can still exercise them.
        if fixture.is_none() && mode != Mode::ClearEngineData {
            ensure_standard_privileges()?;
        }
        let library = Library::open(PathBuf::from(&args[2]))?;
        let id = Uuid::parse_str(args[4].to_str().ok_or("game ID is not UTF-8")?)?;
        let game = library
            .list()?
            .into_iter()
            .find(|game| game.id == id)
            .ok_or("game ID is not in this library")?;
        if mode == Mode::ClearEngineData {
            // Refused while a player holds the game; the lock is released when this exits.
            let _lock = library.lock_game(id)?;
            return clear_engine_data(id);
        }
        let (content, profile) = library.player_paths(id)?;
        let _lock = library.lock_game(id)?;
        if let Some(fixture) = fixture {
            verify_smoke_fixture(&content, fixture)?;
        }
        let blocked = Arc::new(AtomicUsize::new(0));
        let origin = GameOrigin::new(id);
        let proxy_port = if mode.offline() {
            Some(start_sinkhole_proxy(
                Arc::clone(&blocked),
                origin.host.clone(),
            )?)
        } else {
            None
        };
        let csp = if mode.offline() {
            OFFLINE_CSP
        } else {
            NETWORK_CSP
        };

        let entry_url = origin.entry_url();
        let navigation_origin = origin.clone();
        let asset_origin = origin.clone();

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
        // Windows and Linux keep all browser data in this folder. macOS ignores it and uses
        // the per-game WKWebsiteDataStore set up in `macos_guard::configuration`.
        let mut context = WebContext::new(Some(profile.clone()));
        let (title_sender, title_receiver) = mpsc::channel::<String>();
        let mut builder = WebViewBuilder::new_with_web_context(&mut context)
            .with_custom_protocol(origin.scheme.clone(), move |_, request| {
                serve_asset(&content, &asset_origin, csp, request)
            })
            .with_navigation_handler(move |url| {
                if url == "about:blank" {
                    return true;
                }
                let allow = navigation_origin.allows(&url);
                if !allow {
                    eprintln!("GameStrike denied navigation: {url}");
                }
                allow
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
            .with_devtools(false);
        #[cfg(windows)]
        {
            use wry::WebViewBuilderExtWindows;
            let browser_args = match proxy_port {
                Some(port) => offline_browser_args(port),
                None => BASE_BROWSER_ARGS.to_owned(),
            };
            builder = builder
                .with_additional_browser_args(browser_args)
                .with_https_scheme(true)
                .with_browser_accelerator_keys(false)
                .with_default_context_menus(false);
        }
        #[cfg(not(windows))]
        if let Some(port) = proxy_port {
            builder = builder.with_proxy_config(sinkhole_proxy_config(port));
        }
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewBuilderExtMacos;
            builder = builder.with_webview_configuration(macos_guard::configuration(id)?);
        }
        if let Some(fixture) = fixture {
            builder = builder.with_document_title_changed_handler(move |title| {
                if fixture_reported(&title, fixture) {
                    let _ = title_sender.send(title);
                }
            });
        }
        let webview = build_webview(builder, &window)?;
        #[cfg(windows)]
        {
            windows_guard::disable_privileged_webview_features(&webview)?;
            windows_guard::install_native_navigation_guards(&webview, &origin)?;
        }
        #[cfg(target_os = "linux")]
        linux_guard::harden(&webview, &profile)?;
        #[cfg(target_os = "macos")]
        macos_guard::verify(&webview, id, mode.offline())?;

        // Configure handlers before navigating to imported JavaScript.
        webview.load_url(&entry_url)?;
        if !mode.offline() {
            eprintln!(
                "This game's web content can request network resources; launch only games you trust with that access."
            );
        }
        let started = Instant::now();
        let mut observed_at = None;
        // macOS may deliver a few more events after Exit; report the count only once.
        let mut reported = false;
        event_loop.run(move |event, _, control_flow| {
            let _keep_browser_state = (&context, &webview, &_lock, &window);
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
            if exit && !reported {
                reported = true;
                // The launcher reads this line to tell the player what offline mode stopped.
                println!("{BLOCKED_PREFIX}{}", blocked.load(Ordering::SeqCst));
                let _ = io::stdout().flush();
                *control_flow = ControlFlow::Exit;
            }
        })
    }

    /// GTK needs the WebView inside the window's container; the other platforms take the window.
    fn build_webview(
        builder: WebViewBuilder<'_>,
        window: &Window,
    ) -> Result<wry::WebView, Box<dyn Error>> {
        #[cfg(target_os = "linux")]
        {
            use tao::platform::unix::WindowExtUnix;
            use wry::WebViewBuilderExtUnix;
            let container = window
                .default_vbox()
                .ok_or("the player window has no GTK container")?;
            Ok(builder.build_gtk(container)?)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Ok(builder.build(window)?)
        }
    }

    /// Engine data outside the library exists only on macOS; elsewhere the library deletes it.
    #[cfg(not(target_os = "macos"))]
    fn clear_engine_data(_id: Uuid) -> Result<(), Box<dyn Error>> {
        println!("GameStrike engine data is kept in the library on this platform.");
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn clear_engine_data(id: Uuid) -> Result<(), Box<dyn Error>> {
        macos_guard::remove_data_store(id)
    }

    const BLOCKED_PREFIX: &str = "gamestrike-network-blocked=";
    #[cfg(windows)]
    const BASE_BROWSER_ARGS: &str =
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";
    /// Offline pages may use their own origin plus inline, data:, and blob: content.
    const OFFLINE_CSP: &str = "default-src 'self' data: blob: 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval'; object-src 'none'; base-uri 'self'; form-action 'self'";
    /// Network play still blocks plugins and base-URL hijacking.
    const NETWORK_CSP: &str = "object-src 'none'; base-uri 'self'";

    /// All HTTP(S) and WebSocket traffic is sent to a proxy this process owns, including
    /// loopback, and WebRTC may not use UDP outside that proxy. The CSP blocks the same
    /// requests earlier; the proxy covers paths CSP does not, such as WebRTC.
    #[cfg(windows)]
    fn offline_browser_args(port: u16) -> String {
        format!(
            "{BASE_BROWSER_ARGS} --proxy-server=http://127.0.0.1:{port} --proxy-bypass-list=<-loopback> --force-webrtc-ip-handling-policy=disable_non_proxied_udp --webrtc-ip-handling-policy=disable_non_proxied_udp"
        )
    }

    /// WebKit (GTK and macOS) is told to send every connection to the owned sink-hole proxy.
    /// WebRTC is switched off entirely on these platforms, see the platform guards.
    #[cfg(not(windows))]
    fn sinkhole_proxy_config(port: u16) -> wry::ProxyConfig {
        wry::ProxyConfig::Http(wry::ProxyEndpoint {
            host: Ipv4Addr::LOCALHOST.to_string(),
            port: port.to_string(),
        })
    }

    /// Accepts each proxy connection, counts it, and answers 403 without contacting anything.
    /// WebView2 itself contacts Microsoft configuration services, and the runtime asks for a few
    /// URLs on the game's own origin outside the custom protocol. Neither is the game reaching
    /// out, so neither is counted in the summary shown to the player. Both are still blocked.
    fn counts_as_game_request(target: &str, own_host: &str) -> bool {
        let host = target.rsplit_once(':').map_or(target, |(host, _)| host);
        let host = host
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        let host = host.split('/').next().unwrap_or(host).to_ascii_lowercase();
        const RUNTIME: [&str; 6] = [
            "skype.com",
            "microsoft.com",
            "msedge.net",
            "live.com",
            "windows.com",
            "bing.com",
        ];
        !host.is_empty()
            && host != "unknown"
            && host != own_host
            && !RUNTIME
                .iter()
                .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
    }

    fn start_sinkhole_proxy(blocked: Arc<AtomicUsize>, own_host: String) -> io::Result<u16> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut head = [0u8; 512];
                let count = stream.read(&mut head).unwrap_or(0);
                let line = String::from_utf8_lossy(&head[..count]);
                let target = line
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("unknown");
                if counts_as_game_request(target, &own_host) {
                    blocked.fetch_add(1, Ordering::SeqCst);
                }
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

    #[cfg(windows)]
    fn require_medium_integrity(rid: Option<u32>) -> Result<(), &'static str> {
        match rid {
            Some(integrity::MEDIUM_INTEGRITY_RID) => Ok(()),
            Some(_) => Err("GameStrike player requires standard Windows integrity"),
            None => Err("GameStrike player could not verify Windows integrity"),
        }
    }

    #[cfg(windows)]
    fn ensure_standard_privileges() -> Result<(), Box<dyn Error>> {
        ensure_integrity_with(integrity::current_integrity_rid)
    }

    #[cfg(windows)]
    fn ensure_integrity_with(
        probe: impl FnOnce() -> io::Result<u32>,
    ) -> Result<(), Box<dyn Error>> {
        let rid = probe()?;
        require_medium_integrity(Some(rid)).map_err(Into::into)
    }

    /// Refuses root and set-user-ID or set-group-ID identities, the Unix forms of elevation.
    #[cfg(unix)]
    fn require_unprivileged([uid, euid, gid, egid]: [u32; 4]) -> Result<(), &'static str> {
        if uid == 0 || euid == 0 {
            Err("GameStrike player will not run imported games as root")
        } else if uid != euid || gid != egid {
            Err(
                "GameStrike player will not run imported games with a set-user-ID or set-group-ID identity",
            )
        } else {
            Ok(())
        }
    }

    #[cfg(unix)]
    fn ensure_standard_privileges() -> Result<(), Box<dyn Error>> {
        require_unprivileged(privileges::ids()).map_err(Into::into)
    }

    #[cfg(windows)]
    #[allow(unsafe_code)]
    mod windows_guard {
        use super::GameOrigin;
        use std::error::Error;
        use webview2_com::{
            LaunchingExternalUriSchemeEventHandler,
            Microsoft::Web::WebView2::Win32::ICoreWebView2_18, NavigationStartingEventHandler,
            take_pwstr,
        };
        use windows::core::{Interface, PWSTR};
        use wry::WebViewExtWindows;

        pub(super) fn disable_privileged_webview_features(
            webview: &wry::WebView,
        ) -> Result<(), Box<dyn Error>> {
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

        pub(super) fn install_native_navigation_guards(
            webview: &wry::WebView,
            origin: &GameOrigin,
        ) -> Result<(), Box<dyn Error>> {
            let webview = webview.webview();
            let origin = origin.clone();
            let mut token = 0;
            unsafe {
                webview.add_FrameNavigationStarting(
                    &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                        let Some(args) = args else { return Ok(()) };
                        args.SetCancel(true)?;
                        let mut raw = PWSTR::null();
                        args.Uri(&mut raw)?;
                        let uri = take_pwstr(raw);
                        let allow = uri == "about:blank" || origin.allows(&uri);
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
    }

    /// WebKitGTK settings that Wry leaves at their defaults, set explicitly and read back.
    #[cfg(target_os = "linux")]
    mod linux_guard {
        use std::{error::Error, path::Path};
        use webkit2gtk::{SettingsExt, UserContentManagerExt, WebViewExt, WebsiteDataManagerExt};
        use wry::WebViewExtUnix;

        pub(super) fn harden(webview: &wry::WebView, profile: &Path) -> Result<(), Box<dyn Error>> {
            let view = webview.webview();
            let settings =
                WebViewExt::settings(&view).ok_or("WebKitGTK settings are unavailable")?;
            // WebRTC and media capture are off in every mode: WebRTC UDP is not covered by CSP or
            // the HTTP proxy, and WebKitGTK has no setting that limits it to proxied traffic.
            settings.set_enable_webrtc(false);
            settings.set_enable_media_stream(false);
            settings.set_enable_developer_extras(false);
            settings.set_javascript_can_open_windows_automatically(false);
            settings.set_allow_file_access_from_file_urls(false);
            settings.set_allow_universal_access_from_file_urls(false);
            settings.set_allow_top_navigation_to_data_urls(false);
            settings.set_enable_hyperlink_auditing(false);
            settings.set_enable_dns_prefetching(false);
            if settings.enables_webrtc()
                || settings.enables_media_stream()
                || settings.enables_developer_extras()
                || settings.is_javascript_can_open_windows_automatically()
                || settings.allows_file_access_from_file_urls()
                || settings.allows_universal_access_from_file_urls()
            {
                return Err("WebKitGTK did not accept GameStrike's isolation settings".into());
            }

            // Wry always adds a `window.ipc` script, even without an IPC handler. No message
            // handler exists behind it, but the player removes the script so nothing is exposed.
            let manager = view
                .user_content_manager()
                .ok_or("WebKitGTK user content manager is unavailable")?;
            manager.remove_all_scripts();
            manager.remove_all_style_sheets();

            // The per-game data folder must really be the one in use, and persistent.
            let data = view
                .website_data_manager()
                .ok_or("WebKitGTK website data manager is unavailable")?;
            if data.is_ephemeral() {
                return Err("WebKitGTK data store is not persistent".into());
            }
            let base = data
                .base_data_directory()
                .ok_or("WebKitGTK data folder is unknown")?;
            if Path::new(base.as_str()) != profile {
                return Err(
                    format!("WebKitGTK uses {base}, not this game's profile folder").into(),
                );
            }

            view.connect_context_menu(|_, _, _, _| true);
            Ok(())
        }
    }

    /// WKWebView setup. Every check fails the launch rather than running with weaker isolation.
    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    mod macos_guard {
        use objc2::{MainThreadMarker, msg_send, rc::Retained, runtime::NSObjectProtocol, sel};
        use objc2_foundation::{
            NSArray, NSObjectNSKeyValueCoding, NSProcessInfo, NSUUID, ns_string,
        };
        use objc2_web_kit::{WKPreferences, WKWebViewConfiguration, WKWebsiteDataStore};
        use std::error::Error;
        use uuid::Uuid;

        /// Per-identifier data stores (and their proxy settings) need macOS 14.
        const MIN_MACOS_MAJOR: isize = 14;

        pub(super) fn require_supported_macos() -> Result<(), Box<dyn Error>> {
            let version = NSProcessInfo::processInfo().operatingSystemVersion();
            if version.majorVersion < MIN_MACOS_MAJOR {
                return Err(format!(
                    "GameStrike player needs macOS {MIN_MACOS_MAJOR} or later to keep each game's data separate (this is {}.{})",
                    version.majorVersion, version.minorVersion
                )
                .into());
            }
            Ok(())
        }

        fn identifier(id: Uuid) -> Retained<NSUUID> {
            NSUUID::from_bytes(*id.as_bytes())
        }

        /// A configuration with this game's own persistent data store and WebRTC switched off.
        pub(super) fn configuration(
            id: Uuid,
        ) -> Result<Retained<WKWebViewConfiguration>, Box<dyn Error>> {
            let mtm = MainThreadMarker::new().ok_or("the player must run on the main thread")?;
            require_supported_macos()?;
            unsafe {
                let config = WKWebViewConfiguration::new(mtm);
                let store = WKWebsiteDataStore::dataStoreForIdentifier(&identifier(id), mtm);
                if !store.isPersistent() {
                    return Err("the game's WebKit data store is not persistent".into());
                }
                config.setWebsiteDataStore(&store);
                let preferences = config.preferences();
                preferences.setJavaScriptCanOpenWindowsAutomatically(false);
                disable_peer_connections(&preferences)?;
                Ok(config)
            }
        }

        /// WKWebView has no public switch for WebRTC, and its UDP is not covered by CSP or the
        /// proxy. WebKit's own preference is used; if it is missing, the player does not start.
        unsafe fn disable_peer_connections(
            preferences: &WKPreferences,
        ) -> Result<(), Box<dyn Error>> {
            if !preferences.respondsToSelector(sel!(_setPeerConnectionEnabled:))
                || !preferences.respondsToSelector(sel!(_peerConnectionEnabled))
            {
                return Err(
                    "this macOS WebKit cannot switch off WebRTC, so GameStrike will not run games"
                        .into(),
                );
            }
            let enabled: bool = unsafe {
                let _: () = msg_send![preferences, _setPeerConnectionEnabled: false];
                msg_send![preferences, _peerConnectionEnabled]
            };
            if enabled {
                return Err("WebKit kept WebRTC enabled, so GameStrike will not run games".into());
            }
            Ok(())
        }

        /// Confirms the built WebView uses this game's store, has no injected scripts, and, when
        /// offline, routes traffic through exactly one proxy.
        pub(super) fn verify(
            webview: &wry::WebView,
            id: Uuid,
            offline: bool,
        ) -> Result<(), Box<dyn Error>> {
            use wry::WebViewExtMacOS;
            let view = webview.webview();
            unsafe {
                let config = view.configuration();
                let store = config.websiteDataStore();
                let in_use = store.identifier().map(|uuid| uuid.as_bytes());
                if in_use != Some(*id.as_bytes()) || !store.isPersistent() {
                    return Err("the WebView is not using this game's own data store".into());
                }
                if config.userContentController().userScripts().count() != 0 {
                    return Err("a script would be injected into the game".into());
                }
                if offline {
                    let proxies = store
                        .valueForKey(ns_string!("proxyConfigurations"))
                        .and_then(|value| value.downcast::<NSArray>().ok())
                        .map_or(0, |proxies| proxies.count());
                    if proxies != 1 {
                        return Err(
                            "offline mode could not route WebKit traffic to its proxy".into()
                        );
                    }
                }
                let enabled: bool = msg_send![&*config.preferences(), _peerConnectionEnabled];
                if enabled {
                    return Err("WebRTC is enabled in the game's WebView".into());
                }
            }
            Ok(())
        }

        /// Deletes the game's WKWebsiteDataStore. Waits for WebKit, with a time limit.
        /// Deletes every kind of website data (local storage, IndexedDB, cookies, caches, and
        /// the rest) from this game's own WKWebsiteDataStore, through WebKit's public API.
        pub(super) fn remove_data_store(id: Uuid) -> Result<(), Box<dyn Error>> {
            use block2::RcBlock;
            use objc2_app_kit::NSApplication;
            use objc2_foundation::{NSDate, NSRunLoop};
            use std::{
                cell::Cell,
                rc::Rc,
                time::{Duration, Instant},
            };

            require_supported_macos()?;
            let mtm = MainThreadMarker::new()
                .ok_or("clearing WebKit data must run on the main thread")?;
            // WebKit expects an initialized application object, even without windows.
            let _app = NSApplication::sharedApplication(mtm);
            eprintln!("GameStrike is asking WebKit to delete the game's website data.");
            let done = Rc::new(Cell::new(false));
            let sink = Rc::clone(&done);
            let handler = RcBlock::new(move || sink.set(true));
            unsafe {
                let store = WKWebsiteDataStore::dataStoreForIdentifier(&identifier(id), mtm);
                if !store.isPersistent() {
                    return Err("the game's WebKit data store is not persistent".into());
                }
                store.removeDataOfTypes_modifiedSince_completionHandler(
                    &WKWebsiteDataStore::allWebsiteDataTypes(mtm),
                    &NSDate::distantPast(),
                    &handler,
                );
            }
            // WebKit answers on the main queue, so the main run loop is spun until it does.
            let deadline = Instant::now() + Duration::from_secs(30);
            while !done.get() {
                if Instant::now() >= deadline {
                    return Err("WebKit did not finish deleting the game's data in time".into());
                }
                NSRunLoop::currentRunLoop()
                    .runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.05));
            }
            println!("GameStrike deleted the game's WebKit website data.");
            Ok(())
        }
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

    /// True only for the game's own origin as this platform's engine reports it.
    ///
    /// Windows: Wry serves the scheme as `https://<scheme>.localhost` (and may report the raw
    /// scheme form). Linux and macOS: only `<scheme>://localhost`; there an
    /// `https://<scheme>.localhost` URL would be a real network request, so it is refused.
    fn is_game_origin(url: &str, scheme: &str, host: &str) -> bool {
        let Ok(uri) = url.parse::<wry::http::Uri>() else {
            return false;
        };
        let authority = uri.authority().map(|value| value.as_str());
        let custom = uri.scheme_str() == Some(scheme) && authority == Some("localhost");
        if cfg!(windows) {
            (uri.scheme_str() == Some("https") && authority == Some(host)) || custom
        } else {
            custom
        }
    }

    fn serve_asset(
        root: &Path,
        origin: &GameOrigin,
        csp: &'static str,
        request: Request<Vec<u8>>,
    ) -> Response<Cow<'static, [u8]>> {
        let mut response = serve_asset_body(root, origin, request);
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
        // Game files come from local disk. With no HTTP cache entry, the engine has no stale copy
        // to revalidate over the network; every load goes back to the custom protocol handler.
        headers.insert(
            HeaderName::from_static("cache-control"),
            HeaderValue::from_static("no-store"),
        );
        response
    }

    fn serve_asset_body(
        root: &Path,
        origin: &GameOrigin,
        request: Request<Vec<u8>>,
    ) -> Response<Cow<'static, [u8]>> {
        if !origin.allows(&request.uri().to_string()) {
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

        fn game() -> GameOrigin {
            GameOrigin {
                scheme: "game".to_owned(),
                host: "game.localhost".to_owned(),
            }
        }

        /// The game's own URL as this platform's engine requests it.
        fn own(path: &str) -> String {
            if cfg!(windows) {
                format!("https://game.localhost{path}")
            } else {
                format!("game://localhost{path}")
            }
        }

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
                .uri(own("/index.html"))
                .body(Vec::new())
                .unwrap();
            let response = serve_asset(&content, &game(), OFFLINE_CSP, request);
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.body().as_ref(), b"<title>fixture</title>");

            for path in [
                "/%2e%2e/secret.txt",
                "/C:%2fsecret.txt",
                "/CON.txt",
                "/%2fetc%2fpasswd",
                "/..%5csecret.txt",
            ] {
                let request = Request::builder().uri(own(path)).body(Vec::new()).unwrap();
                let response = serve_asset(&content, &game(), OFFLINE_CSP, request);
                assert_ne!(response.status(), StatusCode::OK, "{path}");
                assert!(
                    !response
                        .body()
                        .as_ref()
                        .windows(7)
                        .any(|bytes| bytes == b"private")
                );
            }
        }

        #[cfg(unix)]
        #[test]
        fn asset_handler_refuses_symlinks_out_of_the_content_root() {
            let dir = tempfile::tempdir().unwrap();
            let content = dir.path().join("content");
            fs::create_dir(&content).unwrap();
            fs::write(content.join("index.html"), b"fixture").unwrap();
            fs::write(dir.path().join("secret.txt"), b"private").unwrap();
            std::os::unix::fs::symlink(dir.path().join("secret.txt"), content.join("link.txt"))
                .unwrap();
            let content = fs::canonicalize(content).unwrap();
            let request = Request::builder()
                .uri(own("/link.txt"))
                .body(Vec::new())
                .unwrap();
            let response = serve_asset(&content, &game(), OFFLINE_CSP, request);
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert!(!response.body().as_ref().starts_with(b"private"));
        }

        #[test]
        fn navigation_and_asset_origins_are_exact() {
            assert!(game().allows(&own("/index.html")));
            for url in [
                "https://other.localhost/index.html",
                "https://game.localhost.evil/index.html",
                "https://user@game.localhost/index.html",
                "game://other/index.html",
                "game://localhost.evil/index.html",
                "other://localhost/index.html",
                "file:///C:/fixture.txt",
                "file:///etc/passwd",
                "javascript:alert(1)",
                "ms-appx:///fixture.txt",
                "blob:game://localhost/1234",
            ] {
                assert!(!game().allows(url), "{url}");
            }
            // On WebKit platforms the https form is a real network URL, never the game.
            assert_eq!(
                game().allows("https://game.localhost/index.html"),
                cfg!(windows)
            );
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("index.html"), b"fixture").unwrap();
            let request = Request::builder()
                .uri("https://other.localhost/index.html")
                .body(Vec::new())
                .unwrap();
            assert_eq!(
                serve_asset(dir.path(), &game(), OFFLINE_CSP, request).status(),
                StatusCode::FORBIDDEN
            );
        }

        #[test]
        fn each_game_has_its_own_origin() {
            let one = GameOrigin::new(Uuid::new_v4());
            let two = GameOrigin::new(Uuid::new_v4());
            assert_ne!(one.scheme, two.scheme);
            assert!(one.allows(&one.entry_url()));
            assert!(!one.allows(&two.entry_url()));
            assert!(!two.allows(&one.entry_url()));
            assert!(one.entry_url().ends_with("/index.html"));
        }

        #[test]
        fn player_source_registers_no_host_bridge() {
            // Built from pieces so this test does not match itself.
            let source = include_str!("player.rs");
            for forbidden in [
                ["with_ipc", "_handler"].concat(),
                ["add_Web", "MessageReceived"].concat(),
                ["AddHost", "ObjectToScript"].concat(),
                ["with_initialization", "_script"].concat(),
                ["SetAreHostObjectsAllowed(", "true"].concat(),
                ["SetIsWebMessageEnabled(", "true"].concat(),
                // WebKitGTK and WKWebView script bridges.
                ["register_script", "_message_handler"].concat(),
                ["addScript", "MessageHandler"].concat(),
                ["addUser", "Script"].concat(),
                ["add_", "script("].concat(),
                ["set_enable_webrtc(", "true"].concat(),
                ["_setPeerConnectionEnabled: ", "true"].concat(),
            ] {
                assert!(!source.contains(&forbidden), "{forbidden}");
            }
        }

        #[test]
        fn network_needs_an_explicit_flag_and_unknown_modes_fail() {
            assert_eq!(Mode::parse(Some("--offline")), Some(Mode::Offline));
            assert!(Mode::Offline.offline());
            assert!(!Mode::parse(Some("--allow-network")).unwrap().offline());
            assert!(Mode::parse(Some("--probe-offline")).unwrap().offline());
            assert!(!Mode::parse(Some("--probe-network")).unwrap().offline());
            assert!(Mode::parse(Some("--smoke")).unwrap().offline());
            assert_eq!(
                Mode::parse(Some("--clear-engine-data")),
                Some(Mode::ClearEngineData)
            );
            for flag in [None, Some(""), Some("--network"), Some("--OFFLINE")] {
                assert_eq!(Mode::parse(flag), None, "{flag:?}");
            }
            assert_eq!(Mode::Offline.fixture(), None);
            assert_eq!(Mode::Network.fixture(), None);
            assert_eq!(Mode::ClearEngineData.fixture(), None);
        }

        #[cfg(windows)]
        #[test]
        fn offline_mode_routes_everything_through_the_owned_proxy() {
            let args = offline_browser_args(4321);
            assert!(args.contains("--proxy-server=http://127.0.0.1:4321"));
            assert!(args.contains("--proxy-bypass-list=<-loopback>"));
            assert!(args.contains("--force-webrtc-ip-handling-policy=disable_non_proxied_udp --webrtc-ip-handling-policy=disable_non_proxied_udp"));
            assert!(args.starts_with(BASE_BROWSER_ARGS));
        }

        #[cfg(not(windows))]
        #[test]
        fn offline_mode_routes_everything_through_the_owned_proxy() {
            let wry::ProxyConfig::Http(endpoint) = sinkhole_proxy_config(4321) else {
                panic!("offline mode must use the HTTP sink-hole proxy");
            };
            assert_eq!(endpoint.host, "127.0.0.1");
            assert_eq!(endpoint.port, "4321");
        }

        #[test]
        fn offline_csp_allows_only_the_game_origin() {
            for directive in ["default-src 'self' data: blob:", "object-src 'none'"] {
                assert!(OFFLINE_CSP.contains(directive), "{directive}");
            }
            assert!(!OFFLINE_CSP.contains("http:") && !OFFLINE_CSP.contains('*'));
            assert!(!OFFLINE_CSP.contains("connect-src") && !OFFLINE_CSP.contains("file:"));
        }

        #[test]
        fn sinkhole_proxy_answers_403_and_counts_attempts() {
            let blocked = Arc::new(AtomicUsize::new(0));
            let port =
                start_sinkhole_proxy(Arc::clone(&blocked), "game.localhost".to_owned()).unwrap();
            let mut stream = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
            stream
                .write_all(b"CONNECT example.test:443 HTTP/1.1\r\nHost: example.test\r\n\r\n")
                .unwrap();
            let mut reply = String::new();
            stream.read_to_string(&mut reply).unwrap();
            assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
            assert_eq!(blocked.load(Ordering::SeqCst), 1);
        }

        #[test]
        fn runtime_and_own_origin_requests_are_blocked_but_not_blamed_on_the_game() {
            let own = "gamestrikeabc.localhost";
            assert!(counts_as_game_request("cdn.example.test:443", own));
            assert!(counts_as_game_request("http://127.0.0.1:8080/x", own));
            for target in [
                "config.edge.skype.com:443",
                "gamestrikeabc.localhost:443",
                "unknown",
                "edge.microsoft.com:443",
                "microsoft.com:443",
            ] {
                assert!(!counts_as_game_request(target, own), "{target}");
            }
            assert!(counts_as_game_request("notmicrosoft.com:443", own));
        }

        #[test]
        fn every_asset_response_carries_the_selected_csp() {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("index.html"), b"fixture").unwrap();
            let content = fs::canonicalize(dir.path()).unwrap();
            for (url, csp) in [
                (own("/index.html"), OFFLINE_CSP),
                (own("/missing.js"), OFFLINE_CSP),
                ("https://other.localhost/index.html".to_owned(), NETWORK_CSP),
            ] {
                let request = Request::builder().uri(&url).body(Vec::new()).unwrap();
                let response = serve_asset(&content, &game(), csp, request);
                assert_eq!(
                    response.headers().get("content-security-policy").unwrap(),
                    csp,
                    "{url}"
                );
                assert_eq!(
                    response.headers().get("x-content-type-options").unwrap(),
                    "nosniff"
                );
                assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
            }
        }

        #[cfg(windows)]
        #[test]
        fn integrity_policy_refuses_elevated_and_unknown_levels() {
            assert!(require_medium_integrity(Some(integrity::MEDIUM_INTEGRITY_RID)).is_ok());
            for rid in [0, 0x1000, 0x2100, 0x3000, 0x4000, 0x5000, u32::MAX] {
                assert!(require_medium_integrity(Some(rid)).is_err(), "{rid:#x}");
            }
            assert!(require_medium_integrity(None).is_err());
            let query_error =
                ensure_integrity_with(|| Err(io::Error::other("token query failed"))).unwrap_err();
            assert!(query_error.to_string().contains("token query failed"));
        }

        #[cfg(windows)]
        #[test]
        fn current_windows_token_is_queried_before_launch() {
            let rid = integrity::current_integrity_rid().expect("token integrity query failed");
            eprintln!("GameStrike test process integrity RID: {rid:#x}");
            assert_eq!(
                ensure_standard_privileges().is_ok(),
                rid == integrity::MEDIUM_INTEGRITY_RID
            );
        }

        #[cfg(unix)]
        #[test]
        fn unix_policy_refuses_root_and_setuid_identities() {
            assert!(require_unprivileged([1000, 1000, 1000, 1000]).is_ok());
            for ids in [
                [0, 0, 0, 0],
                [1000, 0, 1000, 1000],
                [0, 1000, 1000, 1000],
                [1000, 1001, 1000, 1000],
                [1000, 1000, 1000, 0],
            ] {
                assert!(require_unprivileged(ids).is_err(), "{ids:?}");
            }
        }

        #[cfg(unix)]
        #[test]
        fn current_unix_identity_is_checked_before_launch() {
            let ids = privileges::ids();
            eprintln!("GameStrike test process ids: {ids:?}");
            assert_eq!(
                ensure_standard_privileges().is_ok(),
                ids[0] != 0 && ids[1] != 0 && ids[0] == ids[1] && ids[2] == ids[3]
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

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    game_player::run()
}
