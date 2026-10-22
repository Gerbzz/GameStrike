//! Native application contracts. These require a permitted Rust runner; never browser evidence.
use gamestrike::application::{
    AppCommand, AppSnapshot, CatalogBackend, GameCard, GameId, LaunchOutcome, LauncherService,
    RunningGame,
};
use std::{
    path::Path,
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Default)]
struct Calls {
    launches: usize,
    refreshes: usize,
    fail_load: bool,
    fail_launch: bool,
    fail_refresh: bool,
    refresh_barrier: Option<Arc<Barrier>>,
    favorite_barrier: Option<Arc<Barrier>>,
    favorite_started: bool,
    second_game: bool,
    launch_barrier: Option<Arc<Barrier>>,
    handoff_process: bool,
    dropped: bool,
    exited: bool,
    favorite: bool,
}

struct FakeBackend(Arc<Mutex<Calls>>);
struct FakeProcess(Arc<Mutex<Calls>>);

impl Drop for FakeBackend {
    fn drop(&mut self) {
        self.0.lock().unwrap().dropped = true;
    }
}

fn id() -> GameId {
    GameId::Html(Uuid::from_u128(1))
}

impl RunningGame for FakeProcess {
    fn poll_exit(&mut self) -> Option<Result<(), String>> {
        self.0.lock().unwrap().exited.then_some(Ok(()))
    }
}

impl CatalogBackend for FakeBackend {
    fn snapshot(&mut self) -> Result<AppSnapshot, String> {
        let calls = self.0.lock().unwrap();
        if calls.fail_load {
            return Err("catalog read failed; preserve prior data".into());
        }
        let mut games = vec![GameCard {
            id: id(),
            title: "Original owned fixture".into(),
            source: "HTML".into(),
            favorite: calls.favorite,
            files_ok: true,
            last_played: None,
            cover: None,
        }];
        if calls.second_game {
            let mut second = games[0].clone();
            second.id = GameId::Html(Uuid::from_u128(2));
            games.push(second);
        }
        Ok(AppSnapshot {
            games,
            ..Default::default()
        })
    }

    fn refresh(&mut self) -> Result<(), String> {
        let (barrier, fail) = {
            let mut calls = self.0.lock().unwrap();
            calls.refreshes += 1;
            (calls.refresh_barrier.clone(), calls.fail_refresh)
        };
        if let Some(barrier) = barrier {
            barrier.wait();
        }
        if fail {
            return Err("store scan failed while old catalog remains readable".into());
        }
        Ok(())
    }

    fn import(&mut self, _source: &Path) -> Result<(), String> {
        Err("fixture imports disabled".into())
    }

    fn set_favorite(&mut self, _id: &GameId, value: bool) -> Result<(), String> {
        let barrier = {
            let mut calls = self.0.lock().unwrap();
            calls.favorite_started = true;
            calls.favorite_barrier.take()
        };
        if let Some(barrier) = barrier {
            barrier.wait();
        }
        self.0.lock().unwrap().favorite = value;
        Ok(())
    }

    fn launch(&mut self, _id: &GameId) -> Result<LaunchOutcome, String> {
        let (barrier, fail, handoff) = {
            let mut calls = self.0.lock().unwrap();
            calls.launches += 1;
            (
                calls.launch_barrier.take(),
                calls.fail_launch,
                calls.handoff_process,
            )
        };
        if let Some(barrier) = barrier {
            barrier.wait();
        }
        if fail {
            Err("rejected invalid launch target".into())
        } else {
            let process = Box::new(FakeProcess(Arc::clone(&self.0)));
            Ok(if handoff {
                LaunchOutcome::HandoffProcess(process)
            } else {
                LaunchOutcome::Tracked(process)
            })
        }
    }
}

fn service() -> (LauncherService, Arc<Mutex<Calls>>) {
    let calls = Arc::new(Mutex::new(Calls::default()));
    let backend = FakeBackend(Arc::clone(&calls));
    let mut service = LauncherService::start(move || Ok(Box::new(backend)));
    until(&mut service, |service| !service.snapshot().refreshing);
    assert_eq!(service.snapshot().games.len(), 1);
    (service, calls)
}

fn until(service: &mut LauncherService, condition: impl Fn(&LauncherService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        service.poll_events();
        if condition(service) {
            return;
        }
        assert!(Instant::now() < deadline, "native worker did not respond");
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn repeated_activation_is_reserved_before_the_worker_starts() {
    let (mut service, calls) = service();
    assert!(service.dispatch(AppCommand::Launch(id())).is_ok());
    assert!(service.dispatch(AppCommand::Launch(id())).is_err());
    until(&mut service, |_| calls.lock().unwrap().launches == 1);
    assert!(service.snapshot().running.contains(&id()));
    assert_eq!(calls.lock().unwrap().launches, 1);
}

#[test]
fn tracked_exit_releases_the_launch_guard() {
    let (mut service, calls) = service();
    service.dispatch(AppCommand::Launch(id())).unwrap();
    until(&mut service, |_| calls.lock().unwrap().launches == 1);
    calls.lock().unwrap().exited = true;
    until(&mut service, |service| {
        !service.snapshot().running.contains(&id())
    });
    assert!(service.dispatch(AppCommand::Launch(id())).is_ok());
}

#[test]
fn failed_launch_reports_failure_and_releases_guard() {
    let (mut service, calls) = service();
    calls.lock().unwrap().fail_launch = true;
    service.dispatch(AppCommand::Launch(id())).unwrap();
    until(&mut service, |service| {
        !service.snapshot().running.contains(&id())
    });
    assert!(
        service
            .snapshot()
            .problems
            .iter()
            .any(|p| p.contains("invalid launch"))
    );
    assert_eq!(calls.lock().unwrap().launches, 1);
}

#[test]
fn failed_refresh_keeps_the_last_visible_catalog() {
    let (mut service, calls) = service();
    calls.lock().unwrap().fail_load = true;
    service.dispatch(AppCommand::Refresh).unwrap();
    until(&mut service, |service| !service.snapshot().refreshing);
    assert_eq!(service.snapshot().games[0].title, "Original owned fixture");
    assert!(
        service
            .snapshot()
            .problems
            .iter()
            .any(|p| p.contains("catalog read failed"))
    );
}

#[test]
fn favorite_mutation_publishes_the_persisted_snapshot() {
    let (mut service, _) = service();
    service
        .dispatch(AppCommand::ToggleFavorite(id(), true))
        .unwrap();
    until(&mut service, |service| service.snapshot().games[0].favorite);
}

#[test]
fn failed_import_is_not_erased_by_a_readable_snapshot() {
    let (mut service, _) = service();
    service
        .dispatch(AppCommand::Import("invalid.fixture".into()))
        .unwrap();
    until(&mut service, |service| {
        service
            .snapshot()
            .problems
            .iter()
            .any(|p| p.contains("imports disabled"))
    });
    assert_eq!(service.snapshot().games.len(), 1);
}

#[test]
fn a_successful_queued_mutation_cannot_erase_the_previous_operation_error() {
    let (mut service, _) = service();
    service
        .dispatch(AppCommand::Import("invalid.fixture".into()))
        .unwrap();
    service
        .dispatch(AppCommand::ToggleFavorite(id(), true))
        .unwrap();
    until(&mut service, |service| service.snapshot().games[0].favorite);
    assert!(
        service
            .snapshot()
            .problems
            .iter()
            .any(|p| p.contains("imports disabled"))
    );
}

#[test]
fn an_older_mutation_snapshot_cannot_clear_a_pending_refresh() {
    let (mut service, calls) = service();
    let favorite = Arc::new(Barrier::new(2));
    let refresh = Arc::new(Barrier::new(2));
    {
        let mut calls = calls.lock().unwrap();
        calls.favorite_barrier = Some(Arc::clone(&favorite));
        calls.refresh_barrier = Some(Arc::clone(&refresh));
    }
    service
        .dispatch(AppCommand::ToggleFavorite(id(), true))
        .unwrap();
    until(&mut service, |_| calls.lock().unwrap().favorite_started);
    service.dispatch(AppCommand::Refresh).unwrap();
    favorite.wait();
    until(&mut service, |service| {
        service.snapshot().games[0].favorite && calls.lock().unwrap().refreshes == 1
    });
    assert!(
        service.snapshot().refreshing,
        "older mutation result cleared newer refresh reservation"
    );
    service.dispatch(AppCommand::Refresh).unwrap();
    refresh.wait();
    service
        .dispatch(AppCommand::ToggleFavorite(id(), false))
        .unwrap();
    until(&mut service, |service| {
        !service.snapshot().games[0].favorite
    });
    assert_eq!(calls.lock().unwrap().refreshes, 1);
}

#[test]
fn failed_scan_remains_visible_even_when_the_prior_catalog_is_readable() {
    let (mut service, calls) = service();
    calls.lock().unwrap().fail_refresh = true;
    service.dispatch(AppCommand::Refresh).unwrap();
    until(&mut service, |service| !service.snapshot().refreshing);
    assert!(
        service
            .snapshot()
            .problems
            .iter()
            .any(|p| p.contains("store scan failed"))
    );
    assert_eq!(service.snapshot().games.len(), 1);
}

#[test]
fn closing_ui_cancels_queued_launches_before_the_backend_runs_them() {
    let barrier = Arc::new(Barrier::new(2));
    let calls = Arc::new(Mutex::new(Calls {
        second_game: true,
        launch_barrier: Some(Arc::clone(&barrier)),
        ..Default::default()
    }));
    let backend = FakeBackend(Arc::clone(&calls));
    let mut service = LauncherService::start(move || Ok(Box::new(backend)));
    until(&mut service, |service| !service.snapshot().refreshing);
    service.dispatch(AppCommand::Launch(id())).unwrap();
    until(&mut service, |_| calls.lock().unwrap().launches == 1);
    service
        .dispatch(AppCommand::Launch(GameId::Html(Uuid::from_u128(2))))
        .unwrap();
    drop(service);
    barrier.wait();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !calls.lock().unwrap().dropped {
        assert!(
            Instant::now() < deadline,
            "worker failed to stop after UI close"
        );
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(calls.lock().unwrap().launches, 1);
}

#[test]
fn unknown_game_cannot_reach_the_launch_backend() {
    let (mut service, calls) = service();
    assert!(
        service
            .dispatch(AppCommand::Launch(GameId::Store("unknown".into())))
            .is_err()
    );
    assert_eq!(calls.lock().unwrap().launches, 0);
}

#[test]
fn synthetic_capture_has_no_worker_or_launch_authority() {
    let mut service = LauncherService::synthetic();
    let first = service.snapshot().games[0].id.clone();
    assert!(service.snapshot().games.len() >= 8);
    assert!(service.dispatch(AppCommand::Launch(first)).is_err());
    assert!(
        service
            .dispatch(AppCommand::Import("owned.html".into()))
            .is_err()
    );
    assert!(service.dispatch(AppCommand::Refresh).is_err());
}

#[test]
fn operation_error_history_has_a_fixed_bound() {
    let mut service = LauncherService::synthetic();
    for number in 0..64 {
        service.report_problem(format!("fixture fault {number}"));
    }
    assert_eq!(service.snapshot().problems.len(), 16);
    assert!(
        service
            .snapshot()
            .problems
            .contains(&"fixture fault 63".to_owned())
    );
}

#[test]
fn successful_opener_exit_does_not_immediately_release_the_handoff_guard() {
    let (mut service, calls) = service();
    {
        let mut calls = calls.lock().unwrap();
        calls.handoff_process = true;
        calls.exited = true;
    }
    service.dispatch(AppCommand::Launch(id())).unwrap();
    until(&mut service, |_| calls.lock().unwrap().launches == 1);
    let observe_until = Instant::now() + Duration::from_millis(120);
    while Instant::now() < observe_until {
        service.poll_events();
        assert!(service.snapshot().running.contains(&id()));
        thread::sleep(Duration::from_millis(1));
    }
    assert!(service.dispatch(AppCommand::Launch(id())).is_err());
    until(&mut service, |service| {
        !service.snapshot().running.contains(&id())
    });
}

#[test]
fn worker_completion_wakes_an_idle_native_ui() {
    let calls = Arc::new(Mutex::new(Calls::default()));
    let wakes = Arc::new(AtomicUsize::new(0));
    let notifier = Arc::clone(&wakes);
    let mut service = LauncherService::start_with_waker(
        move || Ok(Box::new(FakeBackend(calls))),
        Arc::new(move || {
            notifier.fetch_add(1, Ordering::Relaxed);
        }),
    );
    until(&mut service, |service| {
        !service.snapshot().refreshing && wakes.load(Ordering::Relaxed) >= 1
    });
    assert!(wakes.load(Ordering::Relaxed) >= 1);
}
