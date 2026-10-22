//! Trusted local library UI. Imported HTML is never loaded in this WebView.
//!
//! Runs on Windows (WebView2), Linux (WebKitGTK), and macOS (WKWebView). Only the file picker and
//! the `--capture-preview` screenshot differ per platform; see the `platform` module.
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn main() {
    eprintln!("The GameStrike launcher supports Windows, Linux, and macOS only.");
    std::process::exit(1);
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
mod launcher_app {
    use gamestrike::{
        HostOs, ImportOutcome, Library, clear_engine_data,
        stores::{ScanEnv, SnapshotRegistry, Store, StoreLibrary, split_arguments},
    };
    use serde::Deserialize;
    use serde_json::{Value, json};
    use std::{
        borrow::Cow,
        env,
        error::Error,
        ffi::OsString,
        fs,
        io::{BufRead, BufReader},
        path::{Path, PathBuf},
        process::{Child, Command, ExitStatus, Stdio},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::Duration,
    };
    use tao::{
        dpi::LogicalSize,
        event::{Event, WindowEvent},
        event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy},
        window::WindowBuilder,
    };
    use uuid::Uuid;
    use wry::{
        WebContext, WebViewBuilder,
        http::{Request, Response, StatusCode, header},
    };

    #[derive(Debug, Deserialize, PartialEq)]
    #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
    enum CommandMessage {
        Ready {},
        PickAndImport {},
        LaunchGame { id: String, network: bool },
        RenameGame { id: String, title: String },
        ResetSaves { id: String },
        RemoveGame { id: String, keep_saves: bool },
        PickCover { id: String },
        UndoCover { id: String },
        ResetCover { id: String },
        SetFavorite { id: String, favorite: bool },
        SetSource { id: String, source: String },
        // Store libraries and custom programs. The page names games by ID only; what to start
        // comes from GameStrike's own scan or from a program the player picked natively.
        RefreshStores {},
        LaunchStoreGame { id: String },
        OpenInstallFolder { id: String },
        AddProgram {},
        SetProgram { id: String, args: String },
        PickWorkingFolder { id: String },
        RemoveProgram { id: String },
        SetHidden { id: String, hidden: bool },
        SetStoreEnabled { store: String, enabled: bool },
        Quit {},
    }

    enum UiEvent {
        Command(CommandMessage),
        CapturePreview,
        /// Asks the page, up to a limit, whether the staged capture view has finished drawing.
        CaptureCheck(u32),
        CaptureNow,
        CaptureComplete(std::result::Result<(), String>),
        ImportResult(std::result::Result<ImportOutcome, String>),
        /// The one-time search for covers inside older games finished.
        AutoCoversReady,
        /// A rescan of the store libraries finished, with the number of installed store games.
        StoresRefreshed(std::result::Result<usize, String>),
        PlayerExited {
            id: Uuid,
            network: bool,
            report: PlayerReport,
        },
    }

    /// What the launcher learned from a finished player process.
    #[derive(Debug, Default)]
    struct PlayerReport {
        status: Option<ExitStatus>,
        wait_error: Option<String>,
        blocked: Option<usize>,
        error_line: Option<String>,
    }

    /// A named UI state for `--capture-view`. Only names in this list reach the page script.
    /// Every view uses the current PWDS Collection on index.html.
    const CAPTURE_VIEWS: &[&str] = &[
        "library",
        "focus",
        "controller",
        "filters",
        "search",
        "favorites",
        "long",
        "detail",
        "saves",
        "moving",
        "settings",
        "launch",
        "about",
        "boot",
        "boot-0.5",
        "boot-2",
        "boot-4.5",
        "boot-5.5",
        "stores",
        "store-detail",
        "add-menu",
        "store-settings",
    ];

    /// Asks the page whether a staged capture view has finished drawing (fonts, covers, and the 3D
    /// scene). A page without a `ready` check, such as classic.html, is always ready.
    const CAPTURE_READY: &str = "(function () { try { var g = window.GameStrike; return !!g && (typeof g.ready !== 'function' || g.ready() === true); } catch (e) { return false; } })()";

    /// Records which renderer drew the backdrop: WebGL and its quality, the static fallback, or none.
    const CAPTURE_RENDERER: &str = "[document.body.dataset.scene, document.body.dataset.quality, document.body.dataset.sceneReason, document.body.dataset.cradle && 'cradle: ' + document.body.dataset.cradle].filter(Boolean).join(' ') || 'not started'";

    /// How many times a capture asks the page whether it is ready (250 ms apart) before failing.
    const CAPTURE_CHECKS: u32 = 40;

    /// The current PWDS Collection is the only application dashboard.
    fn start_url(_view: CaptureView) -> &'static str {
        "gamestrikeui://localhost/index.html"
    }

    #[derive(Clone, Copy)]
    struct CaptureView(&'static str);

    /// What the native file picker is asked for.
    #[derive(Clone, Copy)]
    enum Pick {
        Game,
        Image,
        /// A program, shortcut, or app to add as a custom game.
        Program,
        /// A working folder for a custom program.
        Folder,
    }

    pub fn run() -> Result<(), Box<dyn Error>> {
        let (root, mut capture_path, window_size, capture_view) = launch_options()?;
        let library = Arc::new(Library::open(&root)?);
        let root = fs::canonicalize(root)?;
        let stores = Arc::new(StoreLibrary::open(&root)?);
        let refreshing = Arc::new(AtomicBool::new(false));
        // Synthetic store fixtures are read only for --capture-preview runs; normal runs read the
        // store launchers' real records on this computer.
        let store_fixtures = capture_path
            .as_ref()
            .and_then(|_| env::var_os("GAMESTRIKE_CAPTURE_STORES"))
            .map(PathBuf::from);
        let event_loop = EventLoopBuilder::<UiEvent>::with_user_event().build();
        start_store_refresh(
            &stores,
            &refreshing,
            store_fixtures.clone(),
            event_loop.create_proxy(),
        );
        let window = WindowBuilder::new()
            .with_title("GameStrike")
            .with_inner_size(window_size)
            .with_min_inner_size(LogicalSize::new(880.0, 620.0))
            // Preview captures run beside the user's work, so they never take focus.
            .with_focused(capture_path.is_none())
            .build(&event_loop)?;
        let proxy = event_loop.create_proxy();
        let background_proxy = event_loop.create_proxy();
        let ui_library = Arc::clone(&library);
        // Games added before covers existed get one look for artwork in their own files.
        {
            let library = Arc::clone(&library);
            let proxy = event_loop.create_proxy();
            thread::spawn(move || {
                for game in library.list().unwrap_or_default() {
                    let _ = library.ensure_auto_cover(game.id);
                }
                let _ = proxy.send_event(UiEvent::AutoCoversReady);
            });
        }
        // UI preferences (style, color, sounds) live with the library, not beside the executable.
        let mut ui_context = WebContext::new(Some(root.join("launcher-ui")));
        #[allow(unused_mut)]
        let mut builder = WebViewBuilder::new_with_web_context(&mut ui_context)
            .with_focused(capture_path.is_none())
            .with_custom_protocol("gamestrikeui".into(), move |_, request| {
                serve_ui(&ui_library, request)
            })
            .with_url(start_url(capture_view))
            .with_navigation_handler(|url| url == "about:blank" || is_ui_origin(&url))
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .with_drag_drop_handler(|_| true)
            .with_devtools(false)
            // The launcher's own synthesized startup and interface sounds may start before the
            // first click. This webview only shows bundled pages; game players are separate.
            .with_autoplay(true)
            .with_ipc_handler(move |request| {
                if let Some(command) = parse_command(&request.uri().to_string(), request.body()) {
                    let _ = proxy.send_event(UiEvent::Command(command));
                }
            });
        #[cfg(windows)]
        {
            use wry::WebViewBuilderExtWindows;
            builder = builder
                .with_https_scheme(true)
                .with_default_context_menus(false);
        }
        let webview = platform::build_webview(builder, &window)?;

        let mut view_prepared = false;
        let mut running: Option<Uuid> = None;
        let mut picking = false;
        event_loop.run(move |event, _, flow| {
            let _keep_alive = (&window, &webview, &ui_context);
            *flow = ControlFlow::Wait;
            match event {
                Event::UserEvent(UiEvent::Command(CommandMessage::Ready {})) => {
                    send_library(&webview, &library, &stores, &refreshing, running);
                    if capture_path.is_some() {
                        let proxy = background_proxy.clone();
                        thread::spawn(move || {
                            // Leaves time for the 3D scene to compile its shaders, even on software GL.
                            thread::sleep(Duration::from_millis(2600));
                            let _ = proxy.send_event(UiEvent::CapturePreview);
                        });
                    }
                }
                Event::UserEvent(UiEvent::CapturePreview) => {
                    let prepare = if view_prepared {
                        None
                    } else {
                        // Every view, including the plain library, skips or freezes the boot animation.
                        Some(format!("window.GameStrike.capture('{}')", capture_view.0))
                    };
                    if let Some(script) = prepare {
                        view_prepared = true;
                        if let Err(error) = webview.evaluate_script(&script) {
                            eprintln!("Could not prepare launcher preview: {error}");
                            std::process::exit(2);
                        }
                        let proxy = background_proxy.clone();
                        thread::spawn(move || {
                            thread::sleep(Duration::from_millis(900));
                            let _ = proxy.send_event(UiEvent::CaptureCheck(0));
                        });
                    } else if let Some(path) = capture_path.take()
                        && let Err(error) = platform::capture_preview(&webview, path, background_proxy.clone())
                    {
                        eprintln!("Could not capture launcher: {error}");
                        std::process::exit(2);
                    }
                }
                Event::UserEvent(UiEvent::CaptureCheck(attempt)) => {
                    // Waits until fonts, covers, and the 3D scene have drawn, so slow runners do not
                    // capture a half-drawn view. classic.html has no check and is captured at once.
                    let proxy = background_proxy.clone();
                    let checked = webview.evaluate_script_with_callback(CAPTURE_READY, move |result| {
                        let ready = result.trim() == "true";
                        let proxy = proxy.clone();
                        thread::spawn(move || {
                            if ready {
                                let _ = proxy.send_event(UiEvent::CaptureNow);
                            } else if attempt >= CAPTURE_CHECKS {
                                let _ = proxy.send_event(UiEvent::CaptureComplete(Err(
                                    "Staged launcher view never became ready".into(),
                                )));
                            } else {
                                thread::sleep(Duration::from_millis(250));
                                let _ = proxy.send_event(UiEvent::CaptureCheck(attempt + 1));
                            }
                        });
                    });
                    if let Err(error) = checked {
                        let _ = background_proxy.send_event(UiEvent::CaptureComplete(Err(format!(
                            "Could not check launcher readiness: {error}"
                        ))));
                    }
                }
                Event::UserEvent(UiEvent::CaptureNow) => {
                    let _ = webview.evaluate_script_with_callback(CAPTURE_RENDERER, |result| {
                        println!("GameStrike scene renderer: {result}")
                    });
                    if let Some(path) = capture_path.take()
                        && let Err(error) = platform::capture_preview(&webview, path, background_proxy.clone())
                    {
                        eprintln!("Could not capture launcher: {error}");
                        std::process::exit(2);
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
                Event::UserEvent(UiEvent::Command(CommandMessage::PickAndImport {})) => {
                    // The picker is modal; a queued second request while it is open is ignored.
                    if picking {
                        return;
                    }
                    picking = true;
                    let picked = platform::pick_file(&window, Pick::Game);
                    match picked {
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
                        Ok(None) => {
                            picking = false;
                            send(&webview, json!({"type":"pick_cancelled"}));
                        }
                        Err(error) => {
                            picking = false;
                            send(&webview, json!({"type":"error", "operation":"import", "message":error}));
                        }
                    }
                }
                Event::UserEvent(UiEvent::ImportResult(result)) => {
                    picking = false;
                    match result {
                        Ok(outcome) => send(&webview, json!({
                            "type":"imported",
                            "game":outcome.game,
                            "already_present":outcome.already_present,
                            "restored_saves":outcome.restored_saves,
                        })),
                        Err(error) => send(
                            &webview,
                            json!({"type":"error", "operation":"import", "message":format!("Import rejected: {error}")}),
                        ),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::LaunchGame { id, network })) => {
                    if running.is_some() {
                        send(&webview, json!({"type":"error", "operation":"launch", "id":id, "message":"A game is already open. Close its window first."}));
                        return;
                    }
                    match launch(&library, &root, &id, network) {
                        Ok((uuid, child)) => {
                            running = Some(uuid);
                            let _ = library.record_play(uuid);
                            send(&webview, json!({"type":"player_started", "id":uuid, "network":network}));
                            send_library(&webview, &library, &stores, &refreshing, running);
                            watch_player(child, uuid, network, background_proxy.clone());
                        }
                        Err(error) => send(&webview, json!({"type":"error", "operation":"launch", "id":id, "message":error})),
                    }
                }
                Event::UserEvent(UiEvent::PlayerExited { id, network, report }) => {
                    if running == Some(id) {
                        running = None;
                    }
                    let (message, ok) = describe_exit(&report, network);
                    send(&webview, json!({"type":"player_exited", "id":id, "message":message, "ok":ok}));
                    send_library(&webview, &library, &stores, &refreshing, running);
                    window.set_focus();
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::RenameGame { id, title })) => {
                    let result = parse_id(&id).and_then(|uuid| {
                        if library.has_game(uuid) {
                            library.rename(uuid, &title).map(|game| game.title)
                        } else {
                            stores.rename(uuid, &title)
                        }
                        .map_err(|error| error.to_string())
                    });
                    match result {
                        Ok(title) => send(&webview, json!({"type":"managed", "id":id, "message":format!("Renamed to {title}.")})),
                        Err(error) => send(&webview, json!({"type":"error", "operation":"manage", "id":id, "message":format!("Could not rename: {error}")})),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::ResetSaves { id })) => {
                    let result = parse_id(&id).and_then(|uuid| {
                        if running == Some(uuid) {
                            return Err("Close the game before clearing its saves.".to_owned());
                        }
                        // On macOS the WebKit data store sits outside the library; it goes first.
                        clear_engine_data(&root, uuid)
                            .and_then(|()| library.reset_saves(uuid))
                            .map_err(|error| error.to_string())
                    });
                    match result {
                        Ok(()) => send(&webview, json!({"type":"managed", "id":id, "message":"Saves cleared. The next launch starts fresh."})),
                        Err(error) => send(&webview, json!({"type":"error", "operation":"manage", "id":id, "message":format!("Could not clear saves: {error}")})),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::RemoveGame { id, keep_saves })) => {
                    let result = parse_id(&id).and_then(|uuid| {
                        if running == Some(uuid) {
                            return Err("Close the game before removing it.".to_owned());
                        }
                        if !keep_saves {
                            clear_engine_data(&root, uuid).map_err(|error| error.to_string())?;
                        }
                        library.remove(uuid, keep_saves).map_err(|error| error.to_string())
                    });
                    match result {
                        Ok(()) => send(&webview, json!({"type":"removed", "id":id, "message": if keep_saves {
                            "Game removed. Its saves return if you add the same file again."
                        } else {
                            "Game and saves removed."
                        }})),
                        Err(error) => send(&webview, json!({"type":"error", "operation":"manage", "id":id, "message":format!("Could not remove: {error}")})),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::PickCover { id })) => {
                    if picking {
                        return;
                    }
                    let Ok(uuid) = parse_id(&id) else { return };
                    picking = true;
                    // The page never supplies a path; the image is decoded with limits in Rust.
                    let result = match platform::pick_file(&window, Pick::Image) {
                        Ok(Some(path)) if library.has_game(uuid) => Some(library.set_cover(uuid, path).map_err(|error| error.to_string())),
                        Ok(Some(path)) => Some(stores.set_cover(uuid, &path).map_err(|error| error.to_string())),
                        Ok(None) => None,
                        Err(error) => Some(Err(error)),
                    };
                    picking = false;
                    match result {
                        Some(Ok(())) => send(&webview, json!({"type":"managed", "id":id, "message":"Cover updated. Undo brings back the previous one."})),
                        Some(Err(error)) => send(&webview, json!({"type":"error", "operation":"cover", "id":id, "message":format!("Could not use that image: {error}")})),
                        None => send(&webview, json!({"type":"pick_cancelled"})),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::UndoCover { id })) => {
                    let result = parse_id(&id).and_then(|uuid| {
                        if library.has_game(uuid) { library.undo_cover(uuid) } else { stores.undo_cover(uuid) }
                            .map_err(|error| error.to_string())
                    });
                    match result {
                        Ok(()) => send(&webview, json!({"type":"managed", "id":id, "message":"Previous cover restored."})),
                        Err(error) => send(&webview, json!({"type":"error", "operation":"cover", "id":id, "message":format!("Could not undo: {error}")})),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::ResetCover { id })) => {
                    let result = parse_id(&id).and_then(|uuid| {
                        if library.has_game(uuid) { library.reset_cover(uuid) } else { stores.reset_cover(uuid) }
                            .map_err(|error| error.to_string())
                    });
                    match result {
                        Ok(()) => send(&webview, json!({"type":"managed", "id":id, "message":"Cover reset. Undo brings it back."})),
                        Err(error) => send(&webview, json!({"type":"error", "operation":"cover", "id":id, "message":format!("Could not reset the cover: {error}")})),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::SetFavorite { id, favorite })) => {
                    let result = parse_id(&id).and_then(|uuid| {
                        if library.has_game(uuid) {
                            library.set_favorite(uuid, favorite).map(|_| ())
                        } else {
                            stores.set_favorite(uuid, favorite)
                        }
                        .map_err(|error| error.to_string())
                    });
                    if let Err(error) = result {
                        send(&webview, json!({"type":"error", "operation":"manage", "id":id, "message":format!("Could not update favorites: {error}")}));
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::SetSource { id, source })) => {
                    let result = parse_id(&id).and_then(|uuid| {
                        library.set_source(uuid, Some(&source)).map_err(|error| error.to_string())
                    });
                    match result {
                        Ok(game) => send(&webview, json!({"type":"managed", "id":id, "message": match game.source {
                            Some(source) => format!("Source set to {source}."),
                            None => "Source cleared.".to_owned(),
                        }})),
                        Err(error) => send(&webview, json!({"type":"error", "operation":"manage", "id":id, "message":format!("Could not set the source: {error}")})),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::Quit {})) => {
                    if running.is_none() {
                        *flow = ControlFlow::Exit;
                    } else {
                        send(&webview, json!({"type":"error", "operation":"quit", "message":"Close the game first."}));
                    }
                }
                Event::UserEvent(UiEvent::AutoCoversReady) => send_library(&webview, &library, &stores, &refreshing, running),
                Event::UserEvent(UiEvent::StoresRefreshed(result)) => {
                    match result {
                        Ok(installed) => send(&webview, json!({"type":"stores_refreshed", "installed":installed})),
                        Err(error) => send(&webview, json!({"type":"error", "operation":"stores", "message":format!("Could not read the store libraries: {error}")})),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::RefreshStores {})) => {
                    if start_store_refresh(&stores, &refreshing, store_fixtures.clone(), background_proxy.clone()) {
                        send_library(&webview, &library, &stores, &refreshing, running);
                    }
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::LaunchStoreGame { id })) => {
                    // Store games are apps the player installed; they start through their store's own
                    // link or program. GameStrike counts the launch but does not track the game.
                    let result = parse_id(&id).and_then(|uuid| {
                        let (launch, store) = stores.launch_target(uuid).map_err(|error| error.to_string())?;
                        platform::start(&launch)?;
                        let _ = stores.record_play(uuid);
                        Ok(store)
                    });
                    match result {
                        Ok(store) => send(&webview, json!({"type":"store_started", "id":id, "message": if store == Store::Custom {
                            "Starting the program.".to_owned()
                        } else {
                            format!("Starting through {}.", store.name())
                        }})),
                        Err(error) => send(&webview, json!({"type":"error", "operation":"launch", "id":id, "message":format!("Could not start the game: {error}")})),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::OpenInstallFolder { id })) => {
                    let result = parse_id(&id)
                        .and_then(|uuid| stores.install_folder(uuid).map_err(|error| error.to_string()))
                        .and_then(|folder| platform::open_folder(&folder));
                    if let Err(error) = result {
                        send(&webview, json!({"type":"error", "operation":"manage", "id":id, "message":format!("Could not open the folder: {error}")}));
                    }
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::AddProgram {})) => {
                    if picking {
                        return;
                    }
                    picking = true;
                    let picked = platform::pick_file(&window, Pick::Program);
                    picking = false;
                    match picked {
                        Ok(Some(path)) => match stores.add_program(&path, None) {
                            Ok(new_id) => send(&webview, json!({"type":"program_added", "id":new_id, "message":"Program added. Set its arguments or working folder in its details."})),
                            Err(error) => send(&webview, json!({"type":"error", "operation":"manage", "message":format!("Could not add that program: {error}")})),
                        },
                        Ok(None) => send(&webview, json!({"type":"pick_cancelled"})),
                        Err(error) => send(&webview, json!({"type":"error", "operation":"manage", "message":error})),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::SetProgram { id, args })) => {
                    let result = parse_id(&id).and_then(|uuid| {
                        if args.chars().count() > 600 {
                            return Err("Use up to 600 characters of arguments.".to_owned());
                        }
                        stores.set_program(uuid, split_arguments(&args), None).map_err(|error| error.to_string())
                    });
                    match result {
                        Ok(()) => send(&webview, json!({"type":"managed", "id":id, "message":"Arguments saved."})),
                        Err(error) => send(&webview, json!({"type":"error", "operation":"manage", "id":id, "message":format!("Could not save the arguments: {error}")})),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::PickWorkingFolder { id })) => {
                    if picking {
                        return;
                    }
                    let Ok(uuid) = parse_id(&id) else { return };
                    picking = true;
                    let picked = platform::pick_file(&window, Pick::Folder);
                    picking = false;
                    let result = match picked {
                        Ok(Some(folder)) => {
                            let args = stores
                                .entries()
                                .into_iter()
                                .find(|entry| entry.id == uuid)
                                .map(|entry| entry.args)
                                .unwrap_or_default();
                            Some(stores.set_program(uuid, args, Some(folder)).map_err(|error| error.to_string()))
                        }
                        Ok(None) => None,
                        Err(error) => Some(Err(error)),
                    };
                    match result {
                        Some(Ok(())) => send(&webview, json!({"type":"managed", "id":id, "message":"Working folder set."})),
                        Some(Err(error)) => send(&webview, json!({"type":"error", "operation":"manage", "id":id, "message":format!("Could not set the working folder: {error}")})),
                        None => send(&webview, json!({"type":"pick_cancelled"})),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::RemoveProgram { id })) => {
                    let result = parse_id(&id).and_then(|uuid| stores.remove_program(uuid).map_err(|error| error.to_string()));
                    match result {
                        Ok(()) => send(&webview, json!({"type":"removed", "id":id, "message":"Program removed from GameStrike. The program itself was not touched."})),
                        Err(error) => send(&webview, json!({"type":"error", "operation":"manage", "id":id, "message":format!("Could not remove: {error}")})),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::SetHidden { id, hidden })) => {
                    let result = parse_id(&id).and_then(|uuid| stores.set_hidden(uuid, hidden).map_err(|error| error.to_string()));
                    match result {
                        Ok(()) => send(&webview, json!({"type":"managed", "id":id, "message": if hidden {
                            "Hidden. It is listed under Hidden in the sidebar."
                        } else {
                            "Shown in the library again."
                        }})),
                        Err(error) => send(&webview, json!({"type":"error", "operation":"manage", "id":id, "message":format!("Could not change it: {error}")})),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::UserEvent(UiEvent::Command(CommandMessage::SetStoreEnabled { store, enabled })) => {
                    let result = Store::from_key(&store)
                        .ok_or_else(|| "unknown store".to_owned())
                        .and_then(|store| stores.set_store_enabled(store, enabled).map_err(|error| error.to_string()));
                    match result {
                        Ok(()) if enabled => {
                            start_store_refresh(&stores, &refreshing, store_fixtures.clone(), background_proxy.clone());
                        }
                        Ok(()) => {}
                        Err(error) => send(&webview, json!({"type":"error", "operation":"stores", "message":format!("Could not change the store setting: {error}")})),
                    }
                    send_library(&webview, &library, &stores, &refreshing, running);
                }
                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => *flow = ControlFlow::Exit,
                _ => {}
            }
        })
    }

    /// Accepts only small JSON commands from the bundled UI origin.
    fn parse_command(origin: &str, body: &str) -> Option<CommandMessage> {
        if !is_ui_origin(origin) || body.len() > 1024 {
            return None;
        }
        serde_json::from_str(body).ok()
    }

    fn parse_id(raw: &str) -> std::result::Result<Uuid, String> {
        Uuid::parse_str(raw).map_err(|_| "Invalid game ID".to_owned())
    }

    fn watch_player(mut child: Child, id: Uuid, network: bool, proxy: EventLoopProxy<UiEvent>) {
        let blocked = Arc::new(Mutex::new(None));
        let error_line = Arc::new(Mutex::new(None));
        let readers: Vec<_> = [
            child
                .stdout
                .take()
                .map(|out| Box::new(out) as Box<dyn std::io::Read + Send>),
            child
                .stderr
                .take()
                .map(|err| Box::new(err) as Box<dyn std::io::Read + Send>),
        ]
        .into_iter()
        .flatten()
        .map(|stream| {
            let blocked = Arc::clone(&blocked);
            let error_line = Arc::clone(&error_line);
            thread::spawn(move || {
                for line in BufReader::new(stream)
                    .lines()
                    .map_while(Result::ok)
                    .take(10_000)
                {
                    if let Some(count) = line.strip_prefix(BLOCKED_PREFIX) {
                        *blocked.lock().unwrap() = count.trim().parse().ok();
                    } else if let Some(error) = player_error(&line) {
                        *error_line.lock().unwrap() = Some(error);
                    }
                }
            })
        })
        .collect();
        thread::spawn(move || {
            let mut report = PlayerReport::default();
            match child.wait() {
                Ok(status) => report.status = Some(status),
                Err(error) => report.wait_error = Some(error.to_string()),
            }
            // Pipes can stay open if the WebView runtime inherited them; wait briefly only.
            let deadline = std::time::Instant::now() + Duration::from_millis(800);
            for reader in readers {
                while !reader.is_finished() && std::time::Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(20));
                }
            }
            report.blocked = *blocked.lock().unwrap();
            report.error_line = error_line.lock().unwrap().clone();
            let _ = proxy.send_event(UiEvent::PlayerExited {
                id,
                network,
                report,
            });
        });
    }

    /// Extracts the message from the player's `Error: ...` exit line.
    fn player_error(line: &str) -> Option<String> {
        let rest = line.strip_prefix("Error: ")?;
        let message = rest
            .split_once("error: \"")
            .and_then(|(_, tail)| tail.split_once('"'))
            .map(|(message, _)| message.to_owned())
            .unwrap_or_else(|| rest.trim_matches('"').to_owned());
        Some(message.chars().take(240).collect())
    }

    fn describe_exit(report: &PlayerReport, network: bool) -> (String, bool) {
        match (&report.status, &report.wait_error) {
            (Some(status), _) if status.success() => {
                let blocked = match (network, report.blocked) {
                    (false, Some(0)) => " Offline mode blocked no network requests.".to_owned(),
                    (false, Some(1)) => " Offline mode blocked 1 network request.".to_owned(),
                    (false, Some(count)) => {
                        format!(
                            " Offline mode blocked {count} network requests. If the game looked incomplete, try Play with network."
                        )
                    }
                    _ => String::new(),
                };
                (
                    format!("Game closed. Your progress is saved on this PC.{blocked}"),
                    true,
                )
            }
            (Some(status), _) => (
                match &report.error_line {
                    Some(error) => format!("The game could not open: {error}."),
                    None => format!("The player stopped unexpectedly ({status})."),
                },
                false,
            ),
            (None, Some(error)) => (format!("Could not track the player: {error}"), false),
            (None, None) => ("Could not track the player.".to_owned(), false),
        }
    }

    /// Library root, optional capture PNG, window size, and capture view.
    type LaunchOptions = (PathBuf, Option<PathBuf>, LogicalSize<f64>, CaptureView);

    fn launch_options() -> Result<LaunchOptions, Box<dyn Error>> {
        let args: Vec<_> = env::args_os().collect();
        let default_size = LogicalSize::new(1250.0, 800.0);
        match args.as_slice() {
            [_] => Ok((
                gamestrike::default_library_root()?,
                None,
                default_size,
                CaptureView("library"),
            )),
            [_, flag, root] if flag == "--library" => {
                Ok((PathBuf::from(root), None, default_size, CaptureView("library")))
            }
            [_, flag, root, capture_flag, output]
                if flag == "--library" && capture_flag == "--capture-preview" =>
            {
                Ok((PathBuf::from(root), Some(PathBuf::from(output)), default_size, CaptureView("library")))
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
                    CaptureView("library"),
                ))
            }
            _ => Err(
                "usage: gamestrike-launcher [--library DIRECTORY [--capture-preview PNG [--capture-view VIEW | --window-size WIDTH HEIGHT]]]".into(),
            ),
        }
    }

    fn parse_capture_view(raw: &OsString) -> Result<CaptureView, Box<dyn Error>> {
        raw.to_str()
            .and_then(|name| CAPTURE_VIEWS.iter().find(|view| **view == name))
            .map(|view| CaptureView(view))
            .ok_or_else(|| {
                format!("capture view must be one of: {}", CAPTURE_VIEWS.join(", ")).into()
            })
    }

    /// Checks and saves a captured PNG. Every platform's capture ends here.
    fn save_png(path: &Path, png: &[u8]) -> std::result::Result<(), String> {
        if png.len() > 32 * 1024 * 1024 {
            return Err("launcher preview exceeds 32 MiB".to_owned());
        }
        if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err("launcher preview is not a PNG".to_owned());
        }
        fs::write(path, png).map_err(|error| error.to_string())
    }

    fn launch(
        library: &Library,
        root: &Path,
        raw_id: &str,
        network: bool,
    ) -> std::result::Result<(Uuid, Child), String> {
        let id = parse_id(raw_id)?;
        library
            .player_paths(id)
            .map_err(|error| error.to_string())?;
        let player = gamestrike::sibling_player().map_err(|error| error.to_string())?;
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
            .arg(if network {
                "--allow-network"
            } else {
                "--offline"
            })
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map(|child| (id, child))
            .map_err(|error| format!("Could not start player: {error}"))
    }

    const BLOCKED_PREFIX: &str = "gamestrike-network-blocked=";

    /// Sends the library: HTML games in "games" (the only kind the earlier styles on classic.html
    /// show), store games and custom programs in "store_games", and each store's status.
    fn send_library(
        webview: &wry::WebView,
        library: &Library,
        stores: &StoreLibrary,
        refreshing: &AtomicBool,
        running: Option<Uuid>,
    ) {
        match library.scan() {
            Ok(scan) => send(
                webview,
                json!({
                    "type":"library",
                    "games":scan.games,
                    "problems":scan.problems,
                    "running":running,
                    "synthetic":library.is_synthetic_test_library(),
                    "store_games":stores.entries(),
                    "stores":stores.statuses(HostOs::current()),
                    "scanning":refreshing.load(Ordering::SeqCst),
                }),
            ),
            Err(error) => send(
                webview,
                json!({"type":"error", "operation":"library", "message":format!("Library could not be read: {error}")}),
            ),
        }
    }

    /// Rescans the store libraries in a background thread; the page hears when it is done.
    /// Returns false if a rescan is already running.
    fn start_store_refresh(
        stores: &Arc<StoreLibrary>,
        refreshing: &Arc<AtomicBool>,
        fixtures: Option<PathBuf>,
        proxy: EventLoopProxy<UiEvent>,
    ) -> bool {
        if refreshing.swap(true, Ordering::SeqCst) {
            return false;
        }
        let stores = Arc::clone(stores);
        let refreshing = Arc::clone(refreshing);
        thread::spawn(move || {
            let result = match &fixtures {
                Some(dir) => SnapshotRegistry::load(&dir.join("registry.json"))
                    .and_then(|registry| stores.refresh(&ScanEnv::for_fixtures(dir, &registry))),
                None => {
                    let registry = platform::registry();
                    stores.refresh(&ScanEnv::for_this_computer(registry.as_ref()))
                }
            }
            .map_err(|error| error.to_string());
            refreshing.store(false, Ordering::SeqCst);
            let _ = proxy.send_event(UiEvent::StoresRefreshed(result));
        });
        true
    }

    /// Starts a program GameStrike does not track. A thread reaps it when it exits.
    fn spawn_detached(mut command: Command) -> std::result::Result<(), String> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("it could not be started ({error})"))?;
        thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    /// Starts a program directly, with its arguments as a list and its working folder (or its own
    /// folder). Linux and macOS use this for programs; Windows also for non-shortcut programs.
    fn start_program(
        path: &Path,
        args: &[String],
        working_dir: Option<&Path>,
    ) -> std::result::Result<(), String> {
        let mut command = Command::new(path);
        command.args(args);
        if let Some(folder) = working_dir.or_else(|| path.parent()) {
            command.current_dir(folder);
        }
        spawn_detached(command)
    }

    fn send(webview: &wry::WebView, payload: Value) {
        // JSON strings stay quoted inside the JS expression; titles are rendered with textContent.
        let script = format!("window.GameStrike.receive({payload});");
        if let Err(error) = webview.evaluate_script(&script) {
            eprintln!("Could not update GameStrike library: {error}");
        }
    }

    /// The launcher's own origin as this platform's engine reports it. Wry maps the scheme to
    /// `https://gamestrikeui.localhost` on Windows only; on Linux and macOS that https URL would
    /// be a network request, so only `gamestrikeui://localhost` is accepted there.
    fn is_ui_origin(url: &str) -> bool {
        let Ok(uri) = url.parse::<wry::http::Uri>() else {
            return false;
        };
        let authority = uri.authority().map(|authority| authority.as_str());
        let custom = uri.scheme_str() == Some("gamestrikeui") && authority == Some("localhost");
        if cfg!(windows) {
            (uri.scheme_str() == Some("https") && authority == Some("gamestrikeui.localhost"))
                || custom
        } else {
            custom
        }
    }

    const JS: &str = "text/javascript; charset=utf-8";
    const WOFF2: &str = "font/woff2";

    /// Every file the launcher page may load. Anything else is a 404.
    static UI_FILES: &[(&str, &[u8], &str)] = &[
        (
            "/index.html",
            include_bytes!("../ui/index.html"),
            "text/html; charset=utf-8",
        ),
        ("/app.js", include_bytes!("../ui/app.js"), JS),
        ("/ui-sounds.js", include_bytes!("../ui/ui-sounds.js"), JS),
        ("/core.js", include_bytes!("../ui/core.js"), JS),
        ("/style-gate.js", include_bytes!("../ui/style-gate.js"), JS),
        // The Blades, Original, and PWDS Orange styles.
        (
            "/classic.html",
            include_bytes!("../ui/classic.html"),
            "text/html; charset=utf-8",
        ),
        ("/classic.js", include_bytes!("../ui/classic.js"), JS),
        (
            "/classic.css",
            include_bytes!("../ui/classic.css"),
            "text/css; charset=utf-8",
        ),
        ("/scene.js", include_bytes!("../ui/scene.js"), JS),
        ("/chrome.js", include_bytes!("../ui/chrome.js"), JS),
        ("/boot.js", include_bytes!("../ui/boot.js"), JS),
        ("/frame.js", include_bytes!("../ui/frame.js"), JS),
        ("/covers.js", include_bytes!("../ui/covers.js"), JS),
        (
            "/styles.css",
            include_bytes!("../ui/styles.css"),
            "text/css; charset=utf-8",
        ),
        (
            "/mark.svg",
            include_bytes!("../assets/branding/gamestrike-mark.svg"),
            "image/svg+xml",
        ),
        (
            "/pwds.svg",
            include_bytes!("../assets/branding/PWDS-logo-orange.svg"),
            "image/svg+xml",
        ),
        // three.js r170 (MIT), vendored; the add-ons import it by relative path.
        (
            "/vendor/three/three.module.min.js",
            include_bytes!("../ui/vendor/three/three.module.min.js"),
            JS,
        ),
        (
            "/vendor/three/postprocessing/EffectComposer.js",
            include_bytes!("../ui/vendor/three/postprocessing/EffectComposer.js"),
            JS,
        ),
        (
            "/vendor/three/postprocessing/MaskPass.js",
            include_bytes!("../ui/vendor/three/postprocessing/MaskPass.js"),
            JS,
        ),
        (
            "/vendor/three/postprocessing/OutputPass.js",
            include_bytes!("../ui/vendor/three/postprocessing/OutputPass.js"),
            JS,
        ),
        (
            "/vendor/three/postprocessing/Pass.js",
            include_bytes!("../ui/vendor/three/postprocessing/Pass.js"),
            JS,
        ),
        (
            "/vendor/three/postprocessing/RenderPass.js",
            include_bytes!("../ui/vendor/three/postprocessing/RenderPass.js"),
            JS,
        ),
        (
            "/vendor/three/postprocessing/ShaderPass.js",
            include_bytes!("../ui/vendor/three/postprocessing/ShaderPass.js"),
            JS,
        ),
        (
            "/vendor/three/postprocessing/UnrealBloomPass.js",
            include_bytes!("../ui/vendor/three/postprocessing/UnrealBloomPass.js"),
            JS,
        ),
        (
            "/vendor/three/shaders/CopyShader.js",
            include_bytes!("../ui/vendor/three/shaders/CopyShader.js"),
            JS,
        ),
        (
            "/vendor/three/shaders/LuminosityHighPassShader.js",
            include_bytes!("../ui/vendor/three/shaders/LuminosityHighPassShader.js"),
            JS,
        ),
        (
            "/vendor/three/shaders/OutputShader.js",
            include_bytes!("../ui/vendor/three/shaders/OutputShader.js"),
            JS,
        ),
        // Nova Square, Rajdhani, and Share Tech Mono (SIL Open Font License 1.1), Latin subsets.
        (
            "/fonts/nova-square-latin-400-normal.woff2",
            include_bytes!("../ui/fonts/nova-square-latin-400-normal.woff2"),
            WOFF2,
        ),
        (
            "/fonts/rajdhani-latin-500-normal.woff2",
            include_bytes!("../ui/fonts/rajdhani-latin-500-normal.woff2"),
            WOFF2,
        ),
        (
            "/fonts/rajdhani-latin-600-normal.woff2",
            include_bytes!("../ui/fonts/rajdhani-latin-600-normal.woff2"),
            WOFF2,
        ),
        (
            "/fonts/rajdhani-latin-700-normal.woff2",
            include_bytes!("../ui/fonts/rajdhani-latin-700-normal.woff2"),
            WOFF2,
        ),
        (
            "/fonts/share-tech-mono-latin-400-normal.woff2",
            include_bytes!("../ui/fonts/share-tech-mono-latin-400-normal.woff2"),
            WOFF2,
        ),
    ];

    fn bundled(path: &str) -> Option<(&'static [u8], &'static str)> {
        let path = if path == "/" { "/index.html" } else { path };
        UI_FILES
            .iter()
            .find(|(name, _, _)| *name == path)
            .map(|(_, body, mime)| (*body, *mime))
    }

    /// Serves the bundled UI files, and `/cover/<game uuid>.png`: a cover PNG that GameStrike
    /// itself encoded into the library. The page names only a game ID, never a path.
    fn serve_ui(library: &Library, request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
        let allowed =
            request.method() == wry::http::Method::GET && is_ui_origin(&request.uri().to_string());
        let cover = allowed
            .then(|| request.uri().path().strip_prefix("/cover/"))
            .flatten()
            .map(|name| read_cover(library, name));
        let (status, body, mime): (StatusCode, Cow<'static, [u8]>, &'static str) = if !allowed {
            (
                StatusCode::FORBIDDEN,
                Cow::Borrowed(&b"denied"[..]),
                "text/plain",
            )
        } else if let Some(cover) = cover {
            match cover {
                Some(png) => (StatusCode::OK, Cow::Owned(png), "image/png"),
                None => (
                    StatusCode::NOT_FOUND,
                    Cow::Borrowed(&b"no cover"[..]),
                    "text/plain",
                ),
            }
        } else {
            match bundled(request.uri().path()) {
                Some((body, mime)) => (StatusCode::OK, Cow::Borrowed(body), mime),
                None => (
                    StatusCode::NOT_FOUND,
                    Cow::Borrowed(&b"not found"[..]),
                    "text/plain",
                ),
            }
        };
        let mut response = Response::new(body);
        *response.status_mut() = status;
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, header::HeaderValue::from_static(mime));
        response.headers_mut().insert(header::HeaderName::from_static("content-security-policy"), header::HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; base-uri 'none'; frame-src 'none'; form-action 'none'; connect-src 'none'; font-src 'self'"));
        response.headers_mut().insert(
            header::HeaderName::from_static("x-content-type-options"),
            header::HeaderValue::from_static("nosniff"),
        );
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("no-store"),
        );
        response
    }

    /// Reads `<uuid>.png` from the library: only a valid game ID, only a regular PNG file of
    /// bounded size that GameStrike wrote.
    fn read_cover(library: &Library, name: &str) -> Option<Vec<u8>> {
        let id = Uuid::parse_str(name.strip_suffix(".png")?).ok()?;
        let path = library.display_cover_file(id)?;
        let metadata = fs::symlink_metadata(&path).ok()?;
        if !metadata.is_file() || metadata.len() > gamestrike::MAX_STORED_COVER_BYTES {
            return None;
        }
        let png = fs::read(path).ok()?;
        png.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
            .then_some(png)
    }

    /// The native file picker, `--capture-preview` screenshots, and WebView placement.
    /// Pickers return only a path; `Library::import` validates the file and ZIP on every platform.
    #[cfg(windows)]
    mod platform {
        use super::{Pick, UiEvent, save_png};
        use gamestrike::stores::{Hive, Launch, Registry};
        use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::Path};
        use windows_sys::Win32::{
            Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS},
            System::Registry::{
                HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY, REG_DWORD,
                REG_EXPAND_SZ, REG_SZ, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW,
            },
            UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        };

        /// Read-only access to the Windows registry for the store scanners.
        pub(super) struct WindowsRegistry;

        pub(super) fn registry() -> Box<dyn Registry> {
            Box::new(WindowsRegistry)
        }

        struct OpenKey(HKEY);

        impl Drop for OpenKey {
            fn drop(&mut self) {
                unsafe {
                    RegCloseKey(self.0);
                }
            }
        }

        fn wide(text: &OsStr) -> Vec<u16> {
            text.encode_wide().chain(std::iter::once(0)).collect()
        }

        fn open_key(hive: Hive, path: &str) -> Option<OpenKey> {
            let root = match hive {
                Hive::LocalMachine => HKEY_LOCAL_MACHINE,
                Hive::CurrentUser => HKEY_CURRENT_USER,
            };
            let path = wide(OsStr::new(path));
            let mut key: HKEY = std::ptr::null_mut();
            let status = unsafe {
                RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ | KEY_WOW64_64KEY, &mut key)
            };
            (status == ERROR_SUCCESS).then_some(OpenKey(key))
        }

        impl Registry for WindowsRegistry {
            fn subkeys(&self, hive: Hive, path: &str) -> Vec<String> {
                let Some(key) = open_key(hive, path) else {
                    return Vec::new();
                };
                let mut names = Vec::new();
                let mut buffer = [0u16; 256];
                for index in 0..20_000u32 {
                    let mut length = buffer.len() as u32;
                    let status = unsafe {
                        RegEnumKeyExW(
                            key.0,
                            index,
                            buffer.as_mut_ptr(),
                            &mut length,
                            std::ptr::null(),
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                        )
                    };
                    if status == ERROR_NO_MORE_ITEMS {
                        break;
                    }
                    if status == ERROR_SUCCESS {
                        names.push(String::from_utf16_lossy(&buffer[..length as usize]));
                    }
                }
                names
            }

            fn string(&self, hive: Hive, path: &str, name: &str) -> Option<String> {
                let key = open_key(hive, path)?;
                let name = wide(OsStr::new(name));
                let mut kind = 0u32;
                let mut size = 0u32;
                let status = unsafe {
                    RegQueryValueExW(
                        key.0,
                        name.as_ptr(),
                        std::ptr::null(),
                        &mut kind,
                        std::ptr::null_mut(),
                        &mut size,
                    )
                };
                if status != ERROR_SUCCESS || size > 64 * 1024 {
                    return None;
                }
                let mut data = vec![0u8; size as usize + 2];
                let mut length = size;
                let status = unsafe {
                    RegQueryValueExW(
                        key.0,
                        name.as_ptr(),
                        std::ptr::null(),
                        &mut kind,
                        data.as_mut_ptr(),
                        &mut length,
                    )
                };
                if status != ERROR_SUCCESS {
                    return None;
                }
                data.truncate(length as usize);
                match kind {
                    REG_SZ | REG_EXPAND_SZ => {
                        let units: Vec<u16> = data
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|pair| u16::from_le_bytes(*pair))
                            .collect();
                        let end = units
                            .iter()
                            .position(|unit| *unit == 0)
                            .unwrap_or(units.len());
                        Some(String::from_utf16_lossy(&units[..end]))
                    }
                    REG_DWORD if data.len() >= 4 => {
                        Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]]).to_string())
                    }
                    _ => None,
                }
            }
        }

        /// Opens a store link, a shortcut, or a folder the way Explorer would.
        fn shell_open(target: &OsStr) -> std::result::Result<(), String> {
            let operation = wide(OsStr::new("open"));
            let file = wide(target);
            let result = unsafe {
                ShellExecuteW(
                    std::ptr::null_mut(),
                    operation.as_ptr(),
                    file.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    SW_SHOWNORMAL,
                )
            };
            let code = result as isize;
            if code > 32 {
                Ok(())
            } else {
                Err(format!("Windows could not open it (code {code})"))
            }
        }

        /// Store links and shortcuts (.lnk, .url) open through the shell; programs start directly.
        pub(super) fn start(launch: &Launch) -> std::result::Result<(), String> {
            gamestrike::stores::check_launch(launch).map_err(|error| error.to_string())?;
            match launch {
                Launch::Uri { uri } => shell_open(OsStr::new(uri)),
                Launch::Program {
                    path,
                    args,
                    working_dir,
                } => {
                    let shortcut =
                        path.extension()
                            .and_then(|ext| ext.to_str())
                            .is_some_and(|ext| {
                                ext.eq_ignore_ascii_case("lnk") || ext.eq_ignore_ascii_case("url")
                            });
                    if shortcut {
                        shell_open(path.as_os_str())
                    } else {
                        super::start_program(path, args, working_dir.as_deref())
                    }
                }
            }
        }

        pub(super) fn open_folder(folder: &Path) -> std::result::Result<(), String> {
            shell_open(folder.as_os_str())
        }

        fn pick_folder(window: &Window) -> std::result::Result<Option<PathBuf>, String> {
            use windows::Win32::{
                Foundation::HWND,
                System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree},
                UI::Shell::{FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH},
            };
            let failed = |error: windows::core::Error| format!("Folder picker failed ({error}).");
            unsafe {
                let dialog: IFileOpenDialog =
                    CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)
                        .map_err(failed)?;
                let options = dialog.GetOptions().map_err(failed)?;
                dialog
                    .SetOptions(options | FOS_PICKFOLDERS)
                    .map_err(failed)?;
                if dialog.Show(Some(HWND(window.hwnd() as *mut _))).is_err() {
                    return Ok(None);
                }
                let item = dialog.GetResult().map_err(failed)?;
                let name = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(failed)?;
                let path = name.to_string();
                CoTaskMemFree(Some(name.0 as *const core::ffi::c_void));
                Ok(Some(PathBuf::from(
                    path.map_err(|_| "the folder name is not valid".to_owned())?,
                )))
            }
        }
        use std::{
            error::Error, ffi::OsString, mem::size_of, os::windows::ffi::OsStringExt, path::PathBuf,
        };
        use tao::{
            event_loop::EventLoopProxy, platform::windows::WindowExtWindows, window::Window,
        };
        use webview2_com::{
            CapturePreviewCompletedHandler,
            Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
        };
        use windows::Win32::System::Com::{
            STREAM_SEEK_SET, StructuredStorage::CreateStreamOnHGlobal,
        };
        use windows_sys::Win32::UI::Controls::Dialogs::{
            CommDlgExtendedError, GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST,
            OFN_NOCHANGEDIR, OFN_NODEREFERENCELINKS, OFN_PATHMUSTEXIST, OPENFILENAMEW,
        };
        use wry::{WebViewBuilder, WebViewExtWindows};

        pub(super) fn build_webview(
            builder: WebViewBuilder<'_>,
            window: &Window,
        ) -> Result<wry::WebView, Box<dyn Error>> {
            Ok(builder.build(window)?)
        }

        pub(super) fn capture_preview(
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
                        if png.len() > 32 * 1024 * 1024 {
                            return Err("launcher preview exceeds 32 MiB".to_owned());
                        }
                    }
                    save_png(&path, &png)
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

        pub(super) fn pick_file(
            window: &Window,
            pick: Pick,
        ) -> std::result::Result<Option<PathBuf>, String> {
            if matches!(pick, Pick::Folder) {
                return pick_folder(window);
            }
            let mut selected = vec![0u16; 32_768];
            let (filter, title) = match pick {
                Pick::Program => (
                    "Programs and shortcuts (*.exe;*.lnk;*.url)\0*.exe;*.lnk;*.url\0\0",
                    "Add a program or shortcut\0",
                ),
                Pick::Folder => ("\0\0", "\0"),
                Pick::Game => (
                    "HTML games (*.html;*.htm;*.zip)\0*.html;*.htm;*.zip\0HTML files (*.html;*.htm)\0*.html;*.htm\0ZIP archives (*.zip)\0*.zip\0\0",
                    "Add a local HTML game\0",
                ),
                Pick::Image => (
                    "Images (*.png;*.jpg;*.jpeg;*.gif;*.webp)\0*.png;*.jpg;*.jpeg;*.gif;*.webp\0\0",
                    "Choose a cover image\0",
                ),
            };
            let filter: Vec<u16> = filter.encode_utf16().collect();
            let title: Vec<u16> = title.encode_utf16().collect();
            let mut dialog = OPENFILENAMEW {
                lStructSize: size_of::<OPENFILENAMEW>() as u32,
                hwndOwner: window.hwnd() as *mut _,
                lpstrFilter: filter.as_ptr(),
                lpstrFile: selected.as_mut_ptr(),
                nMaxFile: selected.len() as u32,
                lpstrTitle: title.as_ptr(),
                // A picked shortcut stays a shortcut, so its own arguments and folder apply.
                Flags: OFN_EXPLORER
                    | OFN_FILEMUSTEXIST
                    | OFN_PATHMUSTEXIST
                    | OFN_NOCHANGEDIR
                    | if matches!(pick, Pick::Program) {
                        OFN_NODEREFERENCELINKS
                    } else {
                        0
                    },
                ..Default::default()
            };
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
    }

    #[cfg(target_os = "linux")]
    mod platform {
        use super::{Pick, UiEvent, save_png};
        use gamestrike::stores::{Launch, NoRegistry, Registry};
        use std::{path::Path, process::Command};

        pub(super) fn registry() -> Box<dyn Registry> {
            Box::new(NoRegistry)
        }

        /// Store links open with xdg-open, .desktop entries with gio, and programs directly.
        pub(super) fn start(launch: &Launch) -> std::result::Result<(), String> {
            gamestrike::stores::check_launch(launch).map_err(|error| error.to_string())?;
            match launch {
                Launch::Uri { uri } => {
                    let mut command = Command::new("xdg-open");
                    command.arg(uri);
                    super::spawn_detached(command)
                }
                Launch::Program {
                    path,
                    args,
                    working_dir,
                } => {
                    if path.extension().is_some_and(|ext| ext == "desktop") {
                        let mut command = Command::new("gio");
                        command.arg("launch").arg(path);
                        super::spawn_detached(command)
                    } else {
                        super::start_program(path, args, working_dir.as_deref())
                    }
                }
            }
        }

        pub(super) fn open_folder(folder: &Path) -> std::result::Result<(), String> {
            let mut command = Command::new("xdg-open");
            command.arg(folder);
            super::spawn_detached(command)
        }
        use gtk::prelude::{FileChooserExt, NativeDialogExt};
        use std::{error::Error, path::PathBuf};
        use tao::{event_loop::EventLoopProxy, platform::unix::WindowExtUnix, window::Window};
        use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};
        use wry::{WebViewBuilder, WebViewBuilderExtUnix, WebViewExtUnix};

        pub(super) fn build_webview(
            builder: WebViewBuilder<'_>,
            window: &Window,
        ) -> Result<wry::WebView, Box<dyn Error>> {
            let container = window
                .default_vbox()
                .ok_or("the launcher window has no GTK container")?;
            Ok(builder.build_gtk(container)?)
        }

        pub(super) fn capture_preview(
            webview: &wry::WebView,
            path: PathBuf,
            proxy: EventLoopProxy<UiEvent>,
        ) -> Result<(), Box<dyn Error>> {
            // WebKitGTK renders the visible library into a cairo surface.
            webview.webview().snapshot(
                SnapshotRegion::Visible,
                SnapshotOptions::NONE,
                None::<&webkit2gtk::gio::Cancellable>,
                move |surface| {
                    let result = (|| -> std::result::Result<(), String> {
                        let surface = surface.map_err(|error| error.to_string())?;
                        let mut png = Vec::new();
                        surface
                            .write_to_png(&mut png)
                            .map_err(|error| error.to_string())?;
                        save_png(&path, &png)
                    })();
                    let _ = proxy.send_event(UiEvent::CaptureComplete(result));
                },
            );
            Ok(())
        }

        pub(super) fn pick_file(
            window: &Window,
            pick: Pick,
        ) -> std::result::Result<Option<PathBuf>, String> {
            if matches!(pick, Pick::Folder) {
                let dialog = gtk::FileChooserNative::new(
                    Some("Choose a working folder"),
                    Some(window.gtk_window()),
                    gtk::FileChooserAction::SelectFolder,
                    Some("Choose"),
                    Some("Cancel"),
                );
                dialog.set_modal(true);
                let picked = (dialog.run() == gtk::ResponseType::Accept)
                    .then(|| dialog.filename())
                    .flatten();
                dialog.hide();
                return Ok(picked);
            }
            let (title, accept, name, patterns): (&str, &str, &str, &[&str]) = match pick {
                Pick::Program | Pick::Folder => {
                    ("Add a program", "Add", "Programs and launchers", &["*"])
                }
                Pick::Game => (
                    "Add a local HTML game",
                    "Add",
                    "HTML games (*.html, *.htm, *.zip)",
                    &["*.html", "*.htm", "*.zip", "*.HTML", "*.HTM", "*.ZIP"],
                ),
                Pick::Image => (
                    "Choose a cover image",
                    "Use image",
                    "Images (*.png, *.jpg, *.gif, *.webp)",
                    &[
                        "*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp", "*.PNG", "*.JPG", "*.JPEG",
                        "*.GIF", "*.WEBP",
                    ],
                ),
            };
            let dialog = gtk::FileChooserNative::new(
                Some(title),
                Some(window.gtk_window()),
                gtk::FileChooserAction::Open,
                Some(accept),
                Some("Cancel"),
            );
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(name));
            for pattern in patterns {
                filter.add_pattern(pattern);
            }
            dialog.add_filter(filter);
            dialog.set_modal(true);
            let picked = (dialog.run() == gtk::ResponseType::Accept)
                .then(|| dialog.filename())
                .flatten();
            dialog.hide();
            Ok(picked)
        }
    }

    #[cfg(target_os = "macos")]
    mod platform {
        use super::{Pick, UiEvent, save_png};
        use gamestrike::stores::{Launch, NoRegistry, Registry};
        use std::{path::Path, process::Command};

        pub(super) fn registry() -> Box<dyn Registry> {
            Box::new(NoRegistry)
        }

        /// Store links and folders open with the open command, app bundles with open -a, and
        /// programs directly.
        pub(super) fn start(launch: &Launch) -> std::result::Result<(), String> {
            gamestrike::stores::check_launch(launch).map_err(|error| error.to_string())?;
            match launch {
                Launch::Uri { uri } => {
                    let mut command = Command::new("open");
                    command.arg(uri);
                    super::spawn_detached(command)
                }
                Launch::Program {
                    path,
                    args,
                    working_dir,
                } => {
                    if path.is_dir() && path.extension().is_some_and(|ext| ext == "app") {
                        let mut command = Command::new("open");
                        command.arg("-a").arg(path);
                        if !args.is_empty() {
                            command.arg("--args").args(args);
                        }
                        super::spawn_detached(command)
                    } else {
                        super::start_program(path, args, working_dir.as_deref())
                    }
                }
            }
        }

        pub(super) fn open_folder(folder: &Path) -> std::result::Result<(), String> {
            let mut command = Command::new("open");
            command.arg(folder);
            super::spawn_detached(command)
        }
        use block2::RcBlock;
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSOpenPanel};
        use objc2_foundation::{NSDictionary, NSError, ns_string};
        use std::{cell::RefCell, error::Error, path::PathBuf};
        use tao::{event_loop::EventLoopProxy, window::Window};
        use wry::{WebViewBuilder, WebViewExtMacOS};

        /// `NSModalResponseOK`.
        const MODAL_RESPONSE_OK: isize = 1;

        pub(super) fn build_webview(
            builder: WebViewBuilder<'_>,
            window: &Window,
        ) -> Result<wry::WebView, Box<dyn Error>> {
            Ok(builder.build(window)?)
        }

        pub(super) fn capture_preview(
            webview: &wry::WebView,
            path: PathBuf,
            proxy: EventLoopProxy<UiEvent>,
        ) -> Result<(), Box<dyn Error>> {
            // WKWebView renders the visible library; AppKit encodes it as PNG.
            let path = RefCell::new(Some(path));
            let handler = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                let Some(path) = path.borrow_mut().take() else {
                    return;
                };
                let result = (|| -> std::result::Result<(), String> {
                    let image = unsafe { image.as_ref() }.ok_or_else(|| {
                        unsafe { error.as_ref() }
                            .map(|error| error.localizedDescription().to_string())
                            .unwrap_or_else(|| "WebKit returned no snapshot".to_owned())
                    })?;
                    let tiff = image
                        .TIFFRepresentation()
                        .ok_or("snapshot has no bitmap data")?;
                    let bitmap = NSBitmapImageRep::imageRepWithData(&tiff)
                        .ok_or("snapshot is unreadable")?;
                    let png = unsafe {
                        bitmap.representationUsingType_properties(
                            NSBitmapImageFileType::PNG,
                            &NSDictionary::new(),
                        )
                    }
                    .ok_or("snapshot could not be encoded as PNG")?;
                    save_png(&path, &png.to_vec())
                })();
                let _ = proxy.send_event(UiEvent::CaptureComplete(result));
            });
            unsafe {
                webview
                    .webview()
                    .takeSnapshotWithConfiguration_completionHandler(None, &handler);
            }
            Ok(())
        }

        pub(super) fn pick_file(
            _window: &Window,
            pick: Pick,
        ) -> std::result::Result<Option<PathBuf>, String> {
            let mtm = MainThreadMarker::new().ok_or("the file picker needs the main thread")?;
            let panel = NSOpenPanel::openPanel(mtm);
            let folder = matches!(pick, Pick::Folder);
            panel.setCanChooseFiles(!folder);
            panel.setCanChooseDirectories(folder);
            panel.setAllowsMultipleSelection(false);
            panel.setMessage(Some(match pick {
                Pick::Game => ns_string!("Add a local HTML game (.html, .htm, or .zip)"),
                Pick::Image => ns_string!("Choose a cover image (PNG, JPEG, GIF, or WebP)"),
                Pick::Program => ns_string!("Add an app or program"),
                Pick::Folder => ns_string!("Choose a working folder"),
            }));
            if panel.runModal() != MODAL_RESPONSE_OK {
                return Ok(None);
            }
            let url = panel.URL().ok_or("the file picker returned no file")?;
            let path = url.path().ok_or("the picked file has no path")?;
            Ok(Some(PathBuf::from(path.to_string())))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The launcher UI URL as this platform's engine requests it.
        fn ui(path: &str) -> String {
            if cfg!(windows) {
                format!("https://gamestrikeui.localhost{path}")
            } else {
                format!("gamestrikeui://localhost{path}")
            }
        }

        #[test]
        fn ui_protocol_serves_only_bundled_paths_on_its_exact_origin() {
            let dir = tempfile::tempdir().unwrap();
            let library = Library::open(dir.path()).unwrap();
            let allowed = Request::builder()
                .uri(ui("/index.html"))
                .body(Vec::new())
                .unwrap();
            let response = serve_ui(&library, allowed);
            assert_eq!(response.status(), StatusCode::OK);
            assert!(response.body().as_ref().starts_with(b"<!doctype html>"));

            let mark = Request::builder()
                .uri(ui("/mark.svg"))
                .body(Vec::new())
                .unwrap();
            let mark_response = serve_ui(&library, mark);
            assert_eq!(mark_response.status(), StatusCode::OK);
            assert_eq!(
                mark_response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .unwrap()
                    .as_bytes(),
                b"image/svg+xml"
            );
            assert!(mark_response.body().as_ref().starts_with(b"<svg"));

            let csp = mark_response
                .headers()
                .get("content-security-policy")
                .unwrap()
                .to_str()
                .unwrap();
            assert!(csp.contains("default-src 'none'") && csp.contains("connect-src 'none'"));

            for url in [
                "https://other.localhost/index.html".to_owned(),
                ui("/fox.png"),
                ui("/../Cargo.toml"),
                "https://gamestrike1234.localhost/index.html".to_owned(),
                "https://gamestrikeui.localhost.evil/index.html".to_owned(),
                "gamestrikeui://localhost.evil/index.html".to_owned(),
                ui("/private.txt"),
            ] {
                let request = Request::builder().uri(&url).body(Vec::new()).unwrap();
                assert_ne!(
                    serve_ui(&library, request).status(),
                    StatusCode::OK,
                    "{url}"
                );
            }
        }

        #[test]
        fn collection_is_the_only_dashboard_entry_and_capture_views_are_unique() {
            for view in CAPTURE_VIEWS {
                assert_eq!(start_url(CaptureView(view)), "gamestrikeui://localhost/index.html");
            }
            for removed in ["green", "blades", "original", "pwds", "boot-blades", "boot-original", "boot-pwds"] {
                assert!(!CAPTURE_VIEWS.contains(&removed), "{removed}");
            }
            let mut names = CAPTURE_VIEWS.to_vec();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), CAPTURE_VIEWS.len(), "capture views are unique");
        }

        #[test]
        fn ui_origin_is_the_platform_form_only() {
            assert!(is_ui_origin(&ui("/index.html")));
            // On WebKit platforms the https form would be a network URL, never the launcher.
            assert_eq!(
                is_ui_origin("https://gamestrikeui.localhost/index.html"),
                cfg!(windows)
            );
            for url in [
                "gamestrikeui://other/index.html",
                "file:///index.html",
                "https://user@gamestrikeui.localhost/index.html",
            ] {
                assert!(!is_ui_origin(url), "{url}");
            }
        }

        #[test]
        fn ipc_accepts_only_known_commands_from_the_ui_origin() {
            let ui = ui("/index.html");
            let id = Uuid::new_v4().to_string();
            assert_eq!(
                parse_command(
                    &ui,
                    &format!(r#"{{"type":"launch_store_game","id":"{id}"}}"#)
                ),
                Some(CommandMessage::LaunchStoreGame { id: id.clone() })
            );
            assert_eq!(
                parse_command(
                    &ui,
                    r#"{"type":"set_store_enabled","store":"ea","enabled":false}"#
                ),
                Some(CommandMessage::SetStoreEnabled {
                    store: "ea".to_owned(),
                    enabled: false
                })
            );
            // A store game is named by ID only; the page can never supply a link or a program.
            for body in [
                format!(
                    r#"{{"type":"launch_store_game","id":"{id}","uri":"https://example.com"}}"#
                ),
                r#"{"type":"add_program","path":"C:\\Windows\\System32\\cmd.exe"}"#.to_owned(),
                format!(r#"{{"type":"open_install_folder","id":"{id}","path":"C:\\"}}"#),
            ] {
                assert_eq!(parse_command(&ui, &body), None, "{body}");
            }
            assert_eq!(
                parse_command(
                    &ui,
                    &format!(r#"{{"type":"launch_game","id":"{id}","network":false}}"#)
                ),
                Some(CommandMessage::LaunchGame {
                    id: id.clone(),
                    network: false
                })
            );
            assert_eq!(
                parse_command(
                    &ui,
                    &format!(r#"{{"type":"remove_game","id":"{id}","keep_saves":true}}"#)
                ),
                Some(CommandMessage::RemoveGame {
                    id: id.clone(),
                    keep_saves: true
                })
            );
            for (origin, body) in [
                // Game origins and look-alikes never reach the launcher.
                (
                    format!(
                        "https://gamestrike{}.localhost/index.html",
                        Uuid::new_v4().simple()
                    ),
                    r#"{"type":"ready"}"#.to_owned(),
                ),
                (
                    "https://gamestrikeui.localhost.evil/".to_owned(),
                    r#"{"type":"ready"}"#.to_owned(),
                ),
                (
                    "file:///C:/index.html".to_owned(),
                    r#"{"type":"ready"}"#.to_owned(),
                ),
                // The UI never supplies a file path, and network must be explicit.
                (
                    ui.to_owned(),
                    r#"{"type":"pick_and_import","path":"C:\\x.zip"}"#.to_owned(),
                ),
                (
                    ui.to_owned(),
                    format!(r#"{{"type":"launch_game","id":"{id}"}}"#),
                ),
                (
                    ui.to_owned(),
                    r#"{"type":"run_shell","command":"calc"}"#.to_owned(),
                ),
                (
                    ui.to_owned(),
                    format!(
                        r#"{{"type":"rename_game","id":"{id}","title":"{}"}}"#,
                        "x".repeat(1100)
                    ),
                ),
                (ui.to_owned(), "not json".to_owned()),
            ] {
                assert_eq!(parse_command(&origin, &body), None, "{origin} {body}");
            }
        }

        #[test]
        fn every_bundled_ui_file_is_served_and_every_module_import_resolves() {
            let dir = tempfile::tempdir().unwrap();
            let library = Library::open(dir.path()).unwrap();
            for (path, body, mime) in UI_FILES {
                let response = serve_ui(
                    &library,
                    Request::builder().uri(ui(path)).body(Vec::new()).unwrap(),
                );
                assert_eq!(response.status(), StatusCode::OK, "{path}");
                assert_eq!(response.body().as_ref(), *body, "{path}");
                assert_eq!(response.headers().get(header::CONTENT_TYPE).unwrap(), mime);
                let csp = response
                    .headers()
                    .get("content-security-policy")
                    .unwrap()
                    .to_str()
                    .unwrap();
                assert!(csp.contains("font-src 'self'") && csp.contains("connect-src 'none'"));
            }
            // The ES modules (scene.js and the vendored three.js add-ons) import only files in
            // this table, by relative path; a missing one would leave the 3D scene unloaded.
            for (path, body, _) in UI_FILES.iter().filter(|(path, _, _)| path.ends_with(".js")) {
                let source = String::from_utf8_lossy(body);
                for quote in ["from '", "from \""] {
                    for piece in source.split(quote).skip(1) {
                        let target = piece.split(['\'', '"']).next().unwrap();
                        assert!(
                            target.starts_with("./") || target.starts_with("../"),
                            "{path} imports {target}"
                        );
                        let mut parts: Vec<&str> = path.split('/').collect();
                        parts.pop();
                        for segment in target.split('/') {
                            match segment {
                                "." => {}
                                ".." => {
                                    parts.pop();
                                }
                                other => parts.push(other),
                            }
                        }
                        let resolved = parts.join("/");
                        assert!(
                            bundled(&resolved).is_some(),
                            "{path} imports missing {resolved}"
                        );
                    }
                }
            }
            assert!(bundled("/vendor/three/three.module.js").is_none());
            assert!(bundled("/fonts/OFL-Orbitron.txt").is_none());
        }

        #[test]
        fn covers_are_served_only_by_game_id_and_only_as_stored_pngs() {
            let dir = tempfile::tempdir().unwrap();
            let library = Library::open(dir.path().join("library")).unwrap();
            let source = dir.path().join("Cover Serve (Synthetic Test).html");
            fs::write(&source, b"<!doctype html><title>synthetic</title>").unwrap();
            let game = library.import(&source).unwrap();
            let get = |path: String| {
                serve_ui(
                    &library,
                    Request::builder().uri(ui(&path)).body(Vec::new()).unwrap(),
                )
            };
            assert_eq!(
                get(format!("/cover/{}.png", game.id)).status(),
                StatusCode::NOT_FOUND,
                "no cover yet"
            );

            let image = dir.path().join("cover.png");
            let mut png = Vec::new();
            image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
                12,
                18,
                image::Rgb([20, 220, 60]),
            ))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
            fs::write(&image, &png).unwrap();
            library.set_cover(game.id, &image).unwrap();
            let response = get(format!("/cover/{}.png?v=2", game.id));
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE).unwrap(),
                "image/png"
            );
            assert_eq!(
                response.headers().get("x-content-type-options").unwrap(),
                "nosniff"
            );
            assert!(response.body().as_ref().starts_with(b"\x89PNG"));

            for path in [
                "/cover/../../Cargo.toml".to_owned(),
                format!("/cover/{}", game.id),
                format!("/cover/{}.svg", game.id),
                format!("/cover/{}.png", Uuid::new_v4()),
                "/cover/not-a-uuid.png".to_owned(),
                "/cover/".to_owned(),
            ] {
                assert_ne!(get(path.clone()).status(), StatusCode::OK, "{path}");
            }
            // Other origins never get covers.
            let foreign = Request::builder()
                .uri(format!(
                    "https://gamestrike{}.localhost/cover/{}.png",
                    game.id.simple(),
                    game.id
                ))
                .body(Vec::new())
                .unwrap();
            assert_eq!(serve_ui(&library, foreign).status(), StatusCode::FORBIDDEN);
        }

        #[test]
        fn library_commands_for_covers_favorites_and_sources_are_strict() {
            let ui = ui("/index.html");
            let id = Uuid::new_v4().to_string();
            assert_eq!(
                parse_command(&ui, &format!(r#"{{"type":"pick_cover","id":"{id}"}}"#)),
                Some(CommandMessage::PickCover { id: id.clone() })
            );
            assert_eq!(
                parse_command(
                    &ui,
                    &format!(r#"{{"type":"set_favorite","id":"{id}","favorite":true}}"#)
                ),
                Some(CommandMessage::SetFavorite {
                    id: id.clone(),
                    favorite: true
                })
            );
            assert_eq!(
                parse_command(
                    &ui,
                    &format!(r#"{{"type":"set_source","id":"{id}","source":"itch.io"}}"#)
                ),
                Some(CommandMessage::SetSource {
                    id: id.clone(),
                    source: "itch.io".to_owned()
                })
            );
            assert_eq!(
                parse_command(&ui, r#"{"type":"quit"}"#),
                Some(CommandMessage::Quit {})
            );
            for body in [
                // The page can never name an image file; only Rust's picker supplies paths.
                format!(r#"{{"type":"pick_cover","id":"{id}","path":"C:\\cover.png"}}"#),
                format!(r#"{{"type":"set_cover","id":"{id}","path":"/etc/passwd"}}"#),
                format!(r#"{{"type":"set_favorite","id":"{id}"}}"#),
                format!(r#"{{"type":"set_favorite","id":"{id}","favorite":"yes"}}"#),
                r#"{"type":"undo_cover"}"#.to_owned(),
            ] {
                assert_eq!(parse_command(&ui, &body), None, "{body}");
            }
            let foreign = format!(
                "https://gamestrike{}.localhost/index.html",
                Uuid::new_v4().simple()
            );
            assert_eq!(
                parse_command(&foreign, &format!(r#"{{"type":"pick_cover","id":"{id}"}}"#)),
                None
            );
        }

        #[test]
        fn capture_views_come_only_from_the_fixed_list() {
            for good in ["detail", "moving", "boot", "library"] {
                assert_eq!(parse_capture_view(&OsString::from(good)).unwrap().0, good);
            }
            for bad in [
                "",
                "LIBRARY",
                "about'); alert(1); ('",
                "collapsed",
                "boot-xbox",
                "classic.html",
                "green",
                "blades",
                "boot-pwds",
                "original-games",
            ] {
                assert!(parse_capture_view(&OsString::from(bad)).is_err(), "{bad}");
            }
        }

        #[test]
        fn player_exit_messages_report_failures_and_offline_blocks() {
            #[cfg(unix)]
            use std::os::unix::process::ExitStatusExt;
            #[cfg(windows)]
            use std::os::windows::process::ExitStatusExt;
            // A wait status of 256 is exit code 1 on Unix; Windows takes the exit code directly.
            let exit_one = if cfg!(windows) { 1 } else { 256 };
            let ok = PlayerReport {
                status: Some(ExitStatus::from_raw(0)),
                blocked: Some(3),
                ..Default::default()
            };
            let (message, success) = describe_exit(&ok, false);
            assert!(
                success && message.contains("blocked 3 network requests"),
                "{message}"
            );
            assert!(!describe_exit(&ok, true).0.contains("blocked"));
            let failed = PlayerReport {
                status: Some(ExitStatus::from_raw(exit_one)),
                error_line: player_error(
                    r#"Error: Custom { kind: InvalidData, error: "this game is already open in a player window" }"#,
                ),
                ..Default::default()
            };
            let (message, success) = describe_exit(&failed, false);
            assert!(!success);
            assert_eq!(
                message,
                "The game could not open: this game is already open in a player window."
            );
            assert_eq!(
                player_error(r#"Error: "GameStrike player requires standard Windows integrity""#)
                    .unwrap(),
                "GameStrike player requires standard Windows integrity"
            );
            assert_eq!(
                player_error("GameStrike offline mode blocked a request"),
                None
            );
        }
    }
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    launcher_app::run()
}
