//! Runs the real `gamestrike-player` window against the bundled isolation probe, on the
//! platform's own engine: WebView2 on Windows, WebKitGTK on Linux, WKWebView on macOS.
//!
//! It opens visible windows (use `xvfb-run` on a headless Linux machine), so it is ignored by default:
//! `cargo test --features launcher --test player_isolation -- --ignored --nocapture`
#![cfg(all(
    feature = "player",
    any(windows, target_os = "linux", target_os = "macos")
))]

use gamestrike::Library;
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::{TcpListener, UdpSocket},
    path::Path,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
use uuid::Uuid;

const PROBE: &[u8] = include_bytes!("../fixtures/isolation-probe.html");
const PORT: u16 = 47391;
/// WebKitGTK and WKWebView: GameStrike switches WebRTC off entirely.
const WEBKIT: bool = cfg!(not(windows));

struct Observer {
    tcp: Arc<AtomicUsize>,
    udp: Arc<AtomicUsize>,
}

impl Observer {
    fn start() -> Self {
        let tcp = Arc::new(AtomicUsize::new(0));
        let udp = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind(("127.0.0.1", PORT)).expect("probe TCP port is busy");
        let tcp_count = Arc::clone(&tcp);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                tcp_count.fetch_add(1, Ordering::SeqCst);
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                let mut buffer = [0u8; 2048];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
            }
        });
        let socket = UdpSocket::bind(("127.0.0.1", PORT)).expect("probe UDP port is busy");
        let udp_count = Arc::clone(&udp);
        thread::spawn(move || {
            let mut buffer = [0u8; 2048];
            while socket.recv_from(&mut buffer).is_ok() {
                udp_count.fetch_add(1, Ordering::SeqCst);
            }
        });
        Self { tcp, udp }
    }

    fn reset(&self) {
        self.tcp.store(0, Ordering::SeqCst);
        self.udp.store(0, Ordering::SeqCst);
    }
}

struct Run {
    results: HashMap<String, String>,
    blocked: usize,
}

fn player(root: &Path, id: Uuid, flag: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_gamestrike-player"))
        .arg("--library")
        .arg(root)
        .args(["--id", &id.to_string(), flag])
        .output()
        .expect("player did not start")
}

fn run_player(root: &Path, id: Uuid, flag: &str) -> Run {
    let output = player(root, id, flag);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("--- player {flag} stdout ---\n{stdout}--- stderr ---\n{stderr}");
    assert!(output.status.success(), "player failed: {}", output.status);
    let title = stdout
        .lines()
        .find_map(|line| line.strip_prefix("GameStrike isolation probe "))
        .expect("probe did not report");
    let results = title
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    let blocked = stdout
        .lines()
        .find_map(|line| line.strip_prefix("gamestrike-network-blocked="))
        .expect("player did not report blocked requests")
        .parse()
        .unwrap();
    Run { results, blocked }
}

fn expect(run: &Run, key: &str, value: &str) {
    expect_any(run, key, &[value]);
}

fn expect_any(run: &Run, key: &str, values: &[&str]) {
    let actual = run.results.get(key).map(String::as_str);
    assert!(
        actual.is_some_and(|actual| values.contains(&actual)),
        "{key}={actual:?}, expected one of {values:?}, in {:?}",
        run.results
    );
}

/// The origin each engine reports for a game: one custom origin per game ID.
fn expected_origin(id: Uuid) -> String {
    if cfg!(windows) {
        format!("https://gamestrike{}.localhost", id.simple())
    } else {
        format!("gamestrike{}://localhost", id.simple())
    }
}

fn expect_offline_isolation(run: &Run, observer: &Observer) {
    expect(run, "self-fetch", "resolved");
    expect(run, "fetch", "rejected");
    expect_any(run, "xhr", &["error", "threw"]);
    expect(run, "img", "error");
    expect(run, "file-fetch", "rejected");
    expect(run, "other-origin", "rejected");
    expect(run, "popup", "null");
    expect(run, "ipc", "undefined");
    // The CSP itself was enforced, independently of the proxy behind it.
    expect(run, "csp", "enforced");
    assert_ne!(run.results.get("ws").map(String::as_str), Some("open"));
    if WEBKIT {
        // No script message handler exists, so WebKit does not even define window.webkit.
        expect(run, "webkit", "undefined");
        expect(run, "webrtc", "unavailable");
    } else {
        // WebView2 always defines window.chrome.webview. The player never registers a
        // WebMessageReceived handler or host object, and disables both settings, so these
        // calls reach nothing. They are reported here rather than asserted.
        eprintln!(
            "chrome.webview: postMessage={:?} hostObjects={:?}",
            run.results.get("postMessage"),
            run.results.get("hostObjects")
        );
    }
    thread::sleep(Duration::from_millis(500));
    assert_eq!(
        observer.tcp.load(Ordering::SeqCst),
        0,
        "offline TCP reached loopback"
    );
    assert_eq!(
        observer.udp.load(Ordering::SeqCst),
        0,
        "offline WebRTC UDP reached loopback"
    );
}

#[test]
#[ignore = "opens real player windows"]
fn imported_games_are_isolated_offline_and_keep_their_own_saves() {
    let observer = Observer::start();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("library");
    let library = Library::open(&root).unwrap();
    let html = dir.path().join("Isolation Probe.html");
    fs::write(&html, PROBE).unwrap();
    let first = library.import(&html).unwrap();

    // A ZIP holding the same probe is a second game with its own origin and profile.
    let zip_path = dir.path().join("Isolation Probe Two.zip");
    {
        let mut zip = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
        zip.start_file("index.html", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(PROBE).unwrap();
        zip.finish().unwrap();
    }
    let second = library.import(&zip_path).unwrap();
    assert_ne!(first.id, second.id);

    for launch in ["1", "2"] {
        observer.reset();
        let run = run_player(&root, first.id, "--probe-offline");
        expect(&run, "launch", launch);
        expect(&run, "idb", launch);
        expect(&run, "origin", &expected_origin(first.id));
        expect_offline_isolation(&run, &observer);
        eprintln!("offline run {launch}: proxy sink counted {}", run.blocked);
    }

    // Positive control: with network allowed, the same probe does reach the listener,
    // so zero offline connections means blocked rather than an unobservable harness.
    observer.reset();
    let run = run_player(&root, second.id, "--probe-network");
    expect(&run, "launch", "1");
    expect(&run, "idb", "1");
    expect(&run, "origin", &expected_origin(second.id));
    expect(&run, "fetch", "resolved");
    expect(&run, "file-fetch", "rejected");
    expect(&run, "other-origin", "rejected");
    expect(&run, "popup", "null");
    expect(&run, "ipc", "undefined");
    thread::sleep(Duration::from_millis(500));
    let tcp = observer.tcp.load(Ordering::SeqCst);
    let udp = observer.udp.load(Ordering::SeqCst);
    eprintln!("network run: tcp={tcp} udp={udp}");
    assert!(tcp > 0, "network-mode control never reached the listener");
    if WEBKIT {
        // WebRTC does not exist for games on WebKit platforms, in either mode, which is why the
        // offline UDP check above cannot be bypassed through it.
        expect(&run, "webrtc", "unavailable");
        assert_eq!(udp, 0, "WebRTC UDP left a WebKit player");
    } else {
        assert!(
            udp > 0,
            "network-mode WebRTC control never sent UDP, so the offline UDP check would prove nothing"
        );
    }

    // The first game's saves were untouched by the second game's run.
    observer.reset();
    let run = run_player(&root, first.id, "--probe-offline");
    expect(&run, "launch", "3");
    expect(&run, "idb", "3");

    // A held lock refuses a second player for the same game, and refuses clearing its data.
    let lock = library.lock_game(first.id).unwrap();
    for flag in ["--probe-offline", "--clear-engine-data"] {
        let refused = player(&root, first.id, flag);
        assert!(!refused.status.success(), "{flag}");
        assert!(
            String::from_utf8_lossy(&refused.stderr).contains("already open"),
            "{flag}"
        );
    }
    drop(lock);

    // Clearing saves really clears them: engine data (outside the library on macOS) first,
    // then the profile folder. The next launch starts from 1 again.
    let cleared = player(&root, first.id, "--clear-engine-data");
    eprintln!(
        "clear-engine-data: {}{}",
        String::from_utf8_lossy(&cleared.stdout),
        String::from_utf8_lossy(&cleared.stderr)
    );
    assert!(cleared.status.success(), "engine data was not cleared");
    library.reset_saves(first.id).unwrap();
    observer.reset();
    let run = run_player(&root, first.id, "--probe-offline");
    expect(&run, "launch", "1");
    expect(&run, "idb", "1");
    expect_offline_isolation(&run, &observer);

    // Leave no WebKit data stores behind on macOS; elsewhere they live in the temporary library.
    for id in [first.id, second.id] {
        assert!(player(&root, id, "--clear-engine-data").status.success());
    }
}

#[test]
fn player_refuses_missing_flags_and_unknown_games() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("library");
    Library::open(&root).unwrap();
    let player = env!("CARGO_BIN_EXE_gamestrike-player");
    let id = Uuid::new_v4().to_string();
    for args in [
        vec!["--library", root.to_str().unwrap(), "--id", &id],
        vec![
            "--library",
            root.to_str().unwrap(),
            "--id",
            &id,
            "--network",
        ],
        vec![
            "--library",
            root.to_str().unwrap(),
            "--id",
            &id,
            "--offline",
        ],
        vec![
            "--library",
            root.to_str().unwrap(),
            "--id",
            &id,
            "--clear-engine-data",
        ],
    ] {
        let output = Command::new(player).args(&args).output().unwrap();
        assert!(!output.status.success(), "{args:?}");
    }
}

#[test]
fn smoke_fixture_refuses_imported_content_that_is_not_the_bundled_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("library");
    let library = Library::open(&root).unwrap();
    let html = dir.path().join("Not The Fixture.html");
    fs::write(
        &html,
        b"<!doctype html><title>Synthetic test, not the fixture</title>",
    )
    .unwrap();
    let game = library.import(&html).unwrap();
    // The check runs before any window opens, so this needs no display.
    let output = player(&root, game.id, "--smoke");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("exact bundled"));
}
