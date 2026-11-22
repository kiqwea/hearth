//! Живые тесты: настоящие API, скачивание Java и ядра, запуск и остановка сервера.
//!
//! По умолчанию пропускаются (нужен интернет и несколько минут). Запуск:
//! `cargo test --test live -- --ignored --test-threads=1 --nocapture`

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hearth_lib::events::{Event, EventSink};
use hearth_lib::manager::{ServerManager, STOP_TIMEOUT};
use hearth_lib::net::Http;
use hearth_lib::paths::Paths;
use hearth_lib::process::ServerStatus;
use hearth_lib::providers::{self, Loader};
use hearth_lib::servers::{CreateServerRequest, Difficulty, GameMode};

#[derive(Default)]
struct PrintSink {
    events: Mutex<Vec<Event>>,
}

impl EventSink for PrintSink {
    fn emit(&self, event: Event) {
        if let Event::Console(c) = &event {
            println!("[{}] {}", c.server_id, c.line.text);
        }
        self.events.lock().unwrap().push(event);
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("0.0.0.0:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn run_server(loader: Loader) {
    // Java и кэш API переживают запуски тестов, серверы каждый раз новые.
    let shared = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("hearth-live");
    let servers = tempfile::tempdir().unwrap();
    let paths = Paths::new(shared.join("config"), shared.join("data"));
    let http = Http::new(paths.cache_dir()).unwrap();
    let sink = Arc::new(PrintSink::default());
    let manager = ServerManager::new(&paths, http.clone(), servers.path().to_path_buf(), sink.clone());

    let versions = providers::list_versions(&http, loader, false).await.unwrap();
    let version = versions.first().expect("есть хотя бы одна версия").id.clone();
    println!("== {loader:?} {version}");

    let meta = manager
        .create(CreateServerRequest {
            name: format!("Тест {loader:?}"),
            loader,
            game_version: version,
            memory_mb: 2048,
            port: Some(free_port()),
            game_mode: GameMode::Creative,
            difficulty: Difficulty::Peaceful,
            max_players: 2,
            seed: Some("hearth".into()),
            eula_accepted: true,
        })
        .await
        .unwrap();
    let id = meta.id.clone();

    manager.start(&id).await.unwrap();
    let started = Instant::now();
    loop {
        match manager.status(&id) {
            ServerStatus::Running => break,
            ServerStatus::Starting => {}
            other => panic!("сервер не запустился: {other:?}"),
        }
        assert!(started.elapsed() < Duration::from_secs(600), "сервер запускается слишком долго");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    println!("== запущен за {:?}", started.elapsed());

    manager.send_command(&id, "/say Привет из Hearth").await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        manager.console(&id).iter().any(|l| l.text.contains("Привет из Hearth")),
        "команда с кириллицей дошла до сервера и вернулась в консоль"
    );

    manager.stop(&id, STOP_TIMEOUT).await.unwrap();
    assert_eq!(manager.status(&id), ServerStatus::Stopped);
    let console = manager.console(&id);
    assert!(
        console.iter().any(|l| l.text.contains("Saving") || l.text.contains("Stopping")),
        "сервер сохранял мир при остановке"
    );
    let level = manager.store().game_dir(&id).join("world").join("level.dat");
    assert!(level.is_file(), "мир сохранён на диск: {}", level.display());

    // Повторный запуск того же сервера: мир и файлы на месте, установка не нужна.
    manager.start(&id).await.unwrap();
    let restarted = Instant::now();
    while manager.status(&id) != ServerStatus::Running {
        assert!(manager.status(&id).is_active(), "повторный запуск упал");
        assert!(restarted.elapsed() < Duration::from_secs(300));
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    manager.stop_all(STOP_TIMEOUT).await;
    assert!(!manager.has_running());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_vanilla() {
    run_server(Loader::Vanilla).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_paper() {
    run_server(Loader::Paper).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_fabric() {
    run_server(Loader::Fabric).await;
}
