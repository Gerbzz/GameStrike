//! Runs the real `gamestrike-player` WebView2 window against the bundled isolation probe.
//!
//! It opens visible windows, so it is ignored by default:
//! `cargo test --features launcher-windows --test player_isolation -- --ignored --nocapture`
#![cfg(all(windows, feature = "player-windows"))]

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

fn run_player(root: &Path, id: Uuid, flag: &str) -> Run {
    let output = Command::new(env!("CARGO_BIN_EXE_gamestrike-player"))
        .args(["--library"])
        .arg(root)
        .args(["--id", &id.to_string(), flag])
        .output()
        .expect("player did not start");
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
    assert_eq!(
        run.results.get(key).map(String::as_str),
        Some(value),
        "{key} in {:?}",
        run.results
    );
}

#[test]
#[ignore = "opens real WebView2 player windows"]
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
        expect(&run, "self-fetch", "resolved");
        expect(&run, "fetch", "rejected");
        expect(&run, "xhr", "error");
        expect(&run, "img", "error");
        expect(&run, "file-fetch", "rejected");
        expect(&run, "other-origin", "rejected");
        expect(&run, "popup", "null");
        expect(&run, "ipc", "undefined");
        expect(&run, "hostObjects", "undefined");
        assert_ne!(run.results.get("ws").map(String::as_str), Some("open"));
        assert_ne!(
            run.results.get("postMessage").map(String::as_str),
            Some("sent")
        );
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
        eprintln!("offline run {launch}: proxy sink counted {}", run.blocked);
    }

    // Positive control: with network allowed, the same probe does reach the listener,
    // so zero offline connections means blocked rather than an unobservable harness.
    observer.reset();
    let run = run_player(&root, second.id, "--probe-network");
    expect(&run, "launch", "1");
    expect(&run, "idb", "1");
    expect(&run, "fetch", "resolved");
    expect(&run, "file-fetch", "rejected");
    expect(&run, "popup", "null");
    expect(&run, "ipc", "undefined");
    thread::sleep(Duration::from_millis(500));
    let tcp = observer.tcp.load(Ordering::SeqCst);
    let udp = observer.udp.load(Ordering::SeqCst);
    eprintln!("network run: tcp={tcp} udp={udp}");
    assert!(tcp > 0, "network-mode control never reached the listener");

    // The first game's saves were untouched by the second game's run.
    observer.reset();
    let run = run_player(&root, first.id, "--probe-offline");
    expect(&run, "launch", "3");
    expect(&run, "idb", "3");

    // A held lock refuses a second player for the same game.
    let lock = library.lock_game(first.id).unwrap();
    let refused = Command::new(env!("CARGO_BIN_EXE_gamestrike-player"))
        .arg("--library")
        .arg(&root)
        .args(["--id", &first.id.to_string(), "--probe-offline"])
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("already open"));
    drop(lock);
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
    ] {
        let output = Command::new(player).args(&args).output().unwrap();
        assert!(!output.status.success(), "{args:?}");
    }
}
