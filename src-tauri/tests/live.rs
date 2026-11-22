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
use hearth_lib::pirate;
use hearth_lib::servers::{CreateServerRequest, Difficulty, GameMode};
use hearth_lib::tunnel::secret::MemoryStore;

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

/// Менеджер с ключом playit в памяти: тесты не должны трогать настоящий аккаунт
/// playit пользователя (из диспетчера учётных данных).
fn test_manager(paths: &Paths, http: &Http, servers: &std::path::Path, sink: Arc<PrintSink>) -> Arc<ServerManager> {
    ServerManager::with_secrets(paths, http.clone(), servers.to_path_buf(), sink, Box::new(MemoryStore::default()))
}

async fn wait_running(manager: &ServerManager, id: &str) {
    let started = Instant::now();
    while manager.status(id) != ServerStatus::Running {
        assert!(manager.status(id).is_active(), "сервер не запустился: {:?}", manager.status(id));
        assert!(started.elapsed() < Duration::from_secs(600), "сервер запускается слишком долго");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Пиратский режим на настоящем сервере: бэкап, плагин авторизации с Modrinth,
/// перенос вещей игрока, запуск с плагином и выключение режима обратно.
async fn run_pirate(loader: Loader, plugin_marker: &str) {
    let shared = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("hearth-live");
    let servers = tempfile::tempdir().unwrap();
    let paths = Paths::new(shared.join("config"), shared.join("data"));
    let http = Http::new(paths.cache_dir()).unwrap();
    let sink = Arc::new(PrintSink::default());
    let manager = test_manager(&paths, &http, servers.path(), sink.clone());
    let version = providers::list_versions(&http, loader, false).await.unwrap()[0].id.clone();
    println!("== пиратский режим {loader:?} {version}");
    let id = manager
        .create(CreateServerRequest {
            name: format!("Пираты {loader:?}"),
            loader,
            game_version: version,
            memory_mb: 2048,
            port: Some(free_port()),
            game_mode: GameMode::Creative,
            difficulty: Difficulty::Peaceful,
            max_players: 2,
            seed: Some("hearth".into()),
            eula_accepted: true,
            pirate_mode: false,
            nick_protection: false,
        })
        .await
        .unwrap()
        .id;

    // Первый запуск в обычном режиме: создаётся мир.
    manager.start(&id).await.unwrap();
    wait_running(&manager, &id).await;
    manager.stop(&id, STOP_TIMEOUT).await.unwrap();

    // Будто на сервере уже играл лицензионный Notch.
    let game = manager.store().game_dir(&id);
    let online = "069a79f4-44e9-4726-a5be-fca90e38aaf5";
    std::fs::write(game.join("usercache.json"), format!(r#"[{{"name":"Notch","uuid":"{online}","expiresOn":"2099-01-01 00:00:00 +0000"}}]"#)).unwrap();
    std::fs::create_dir_all(game.join("world/playerdata")).unwrap();
    std::fs::write(game.join(format!("world/playerdata/{online}.dat")), b"inventory").unwrap();

    let on = manager.set_pirate_mode(&id, true, true).await.unwrap();
    println!("== включено: {:?} бэкап {:?} перенесено {}", on.auth, on.backup, on.migrated);
    assert!(on.auth.is_some(), "плагин авторизации поставлен");
    assert!(on.backup.as_deref().is_some_and(|b| std::path::Path::new(b).is_file()), "бэкап сделан");
    assert_eq!(on.migrated, 1);
    let offline = pirate::offline_uuid("Notch");
    assert!(game.join(format!("world/playerdata/{offline}.dat")).is_file(), "вещи Notch перенесены");
    let props = std::fs::read_to_string(game.join("server.properties")).unwrap();
    assert!(props.contains("online-mode=false") && props.contains("enforce-secure-profile=false"));

    // Сервер запускается с плагином авторизации.
    manager.start(&id).await.unwrap();
    wait_running(&manager, &id).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let console = manager.console(&id);
    assert!(
        console.iter().any(|l| l.text.to_lowercase().contains(plugin_marker)),
        "в логе видно, что {plugin_marker} загрузился"
    );
    manager.stop(&id, STOP_TIMEOUT).await.unwrap();

    if loader == Loader::Paper {
        // Со второго запуска AuthMe говорит по-русски.
        manager.start(&id).await.unwrap();
        wait_running(&manager, &id).await;
        manager.stop(&id, STOP_TIMEOUT).await.unwrap();
        assert!(manager.get(&id).unwrap().meta.auth_lang_set);
        let cfg = std::fs::read_to_string(game.join("plugins/AuthMe/config.yml")).unwrap();
        assert!(cfg.contains("messagesLanguage: ru"));
    }

    // Обратно: плагин убран, online-mode включён, вещи Notch вернулись на лицензионный UUID.
    let off = manager.set_pirate_mode(&id, false, false).await.unwrap();
    assert!(off.meta.auth_plugin.is_none() && !off.meta.pirate_mode);
    assert_eq!(off.migrated, 1, "вещи вернулись на лицензионный UUID (Mojang API)");
    assert!(game.join(format!("world/playerdata/{online}.dat")).is_file());
    let props = std::fs::read_to_string(game.join("server.properties")).unwrap();
    assert!(props.contains("online-mode=true"));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_pirate_paper() {
    run_pirate(Loader::Paper, "authme").await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_pirate_fabric() {
    run_pirate(Loader::Fabric, "easyauth").await;
}

/// Пиратский сервер прямо из мастера: защита ников ставится вместе с сервером,
/// а её отдельный переключатель убирает и возвращает плагин.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_pirate_from_wizard() {
    let shared = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("hearth-live");
    let servers = tempfile::tempdir().unwrap();
    let paths = Paths::new(shared.join("config"), shared.join("data"));
    let http = Http::new(paths.cache_dir()).unwrap();
    let manager = test_manager(&paths, &http, servers.path(), Arc::new(PrintSink::default()));
    let version = providers::list_versions(&http, Loader::Paper, false).await.unwrap()[0].id.clone();
    let id = manager
        .create(CreateServerRequest {
            name: "Пираты из мастера".into(),
            loader: Loader::Paper,
            game_version: version,
            memory_mb: 2048,
            port: Some(free_port()),
            game_mode: GameMode::Creative,
            difficulty: Difficulty::Peaceful,
            max_players: 2,
            seed: None,
            eula_accepted: true,
            pirate_mode: true,
            nick_protection: true,
        })
        .await
        .unwrap()
        .id;
    manager.start(&id).await.unwrap();
    wait_running(&manager, &id).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(manager.console(&id).iter().any(|l| l.text.contains("AuthMe")), "AuthMe загрузился с первого запуска");
    manager.stop(&id, STOP_TIMEOUT).await.unwrap();

    let plugins = manager.store().game_dir(&id).join("plugins");
    let jar = manager.get(&id).unwrap().meta.auth_plugin.expect("плагин записан в server.json");
    assert!(plugins.join(&jar).is_file());

    // Маленькой компании защита мешает: выключаем, jar убран, пароли (папка AuthMe) остались.
    let meta = manager.set_nick_protection(&id, false).await.unwrap();
    assert!(!meta.nick_protection && meta.auth_plugin.is_none() && meta.pirate_mode);
    assert!(!plugins.join(&jar).exists() && plugins.join("AuthMe").is_dir());
    // И обратно.
    let meta = manager.set_nick_protection(&id, true).await.unwrap();
    assert!(meta.nick_protection && plugins.join(meta.auth_plugin.unwrap()).is_file());
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
    let manager = test_manager(&paths, &http, servers.path(), sink.clone());

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
            pirate_mode: false,
            nick_protection: false,
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
