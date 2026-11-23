//! Живые тесты: настоящие API, скачивание Java и ядра, запуск и остановка сервера.
//!
//! По умолчанию пропускаются (нужен интернет и несколько минут). Запуск:
//! `cargo test --test live -- --ignored --test-threads=1 --nocapture`

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hearth_lib::backups::BackupKind;
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

/// Статистика и управление игроками на настоящем сервере (через RCON).
async fn run_players(loader: Loader) {
    let shared = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("hearth-live");
    let servers = tempfile::tempdir().unwrap();
    let paths = Paths::new(shared.join("config"), shared.join("data"));
    let http = Http::new(paths.cache_dir()).unwrap();
    let manager = test_manager(&paths, &http, servers.path(), Arc::new(PrintSink::default()));
    let version = providers::list_versions(&http, loader, false).await.unwrap()[0].id.clone();
    let id = manager
        .create(CreateServerRequest {
            name: format!("Игроки {loader:?}"),
            loader,
            game_version: version.clone(),
            memory_mb: 2048,
            port: Some(free_port()),
            game_mode: GameMode::Creative,
            difficulty: Difficulty::Peaceful,
            max_players: 7,
            seed: None,
            eula_accepted: true,
            pirate_mode: false,
            nick_protection: false,
        })
        .await
        .unwrap()
        .id;
    manager.start(&id).await.unwrap();
    wait_running(&manager, &id).await;

    // Статистика: память, процессор, слоты и TPS приходят в течение пары опросов.
    let started = Instant::now();
    let stats = loop {
        let s = manager.get(&id).unwrap().stats;
        if let Some(s) = s.filter(|s| s.max_players.is_some() && s.tps.is_some()) {
            break s;
        }
        if started.elapsed() > Duration::from_secs(30) {
            let props = std::fs::read_to_string(manager.store().game_dir(&id).join("server.properties")).unwrap();
            for line in props.lines().filter(|l| l.contains("permission") || l.contains("rcon") || l.contains("op-")) {
                println!("== prop {line}");
            }
            for cmd in ["list", "help", "minecraft:list", "minecraft:tick query", "tps"] {
                println!("== RCON {cmd:?} → {:?}", manager.rcon(&id, cmd).await);
            }
            panic!("статистика не пришла: {:?}", manager.get(&id).unwrap().stats);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    println!("== {loader:?} {version}: {stats:?}");
    assert_eq!(stats.max_players, Some(7));
    assert!(stats.online.is_empty());
    assert!(stats.memory_mb.unwrap_or(0) > 100, "память процесса посчитана");
    let tps = stats.tps.unwrap();
    assert!((1.0..=20.0).contains(&tps), "TPS разобран: {tps}");

    // Белый список, оператор, бан и разбан через RCON.
    use hearth_lib::players::PlayerAction::*;
    for action in [WhitelistAdd, Op, Ban] {
        let reply = manager.player_action(&id, action, "Notch", Some("тест")).await.unwrap();
        println!("== {action:?}: {reply}");
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let notch = manager.players(&id).unwrap().into_iter().find(|p| p.name == "Notch").expect("Notch в списке игроков");
    assert!(notch.whitelisted && notch.op && notch.banned, "{notch:?}");
    assert_eq!(notch.ban_reason.as_deref(), Some("тест"));
    manager.player_action(&id, Pardon, "Notch", None).await.unwrap();
    manager.player_action(&id, Deop, "Notch", None).await.unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let notch = manager.players(&id).unwrap().into_iter().find(|p| p.name == "Notch").unwrap();
    assert!(!notch.banned && !notch.op && notch.whitelisted, "{notch:?}");
    assert!(manager.player_action(&id, Kick, "bad name!", None).await.is_err(), "некорректный ник отклонён");

    manager.stop(&id, STOP_TIMEOUT).await.unwrap();
    assert!(manager.get(&id).unwrap().stats.is_none(), "статистика сброшена после остановки");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_players_paper() {
    run_players(Loader::Paper).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_players_vanilla() {
    run_players(Loader::Vanilla).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_players_fabric() {
    run_players(Loader::Fabric).await;
}

fn zip_names(path: &std::path::Path) -> Vec<String> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().unwrap().to_string()).collect()
}

/// Миры и копии на настоящем сервере: копия и экспорт на работающем сервере
/// (save-off → save-all flush → zip → save-on), автовыключение пустого сервера,
/// восстановление, импорт мира в формате одиночной игры и запуск с ним.
async fn run_backups(loader: Loader) {
    let shared = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("hearth-live");
    let servers = tempfile::tempdir().unwrap();
    let paths = Paths::new(shared.join("config"), shared.join("data"));
    let http = Http::new(paths.cache_dir()).unwrap();
    let manager = test_manager(&paths, &http, servers.path(), Arc::new(PrintSink::default()));
    let version = providers::list_versions(&http, loader, false).await.unwrap()[0].id.clone();
    println!("== бэкапы {loader:?} {version}");
    let id = manager
        .create(CreateServerRequest {
            name: format!("Бэкапы {loader:?}"),
            loader,
            game_version: version,
            memory_mb: 2048,
            port: Some(free_port()),
            game_mode: GameMode::Creative,
            difficulty: Difficulty::Peaceful,
            max_players: 3,
            seed: Some("hearth".into()),
            eula_accepted: true,
            pirate_mode: false,
            nick_protection: false,
        })
        .await
        .unwrap()
        .id;
    let meta = manager.store().load(&id).unwrap();
    assert!(meta.auto_backup.enabled && meta.auto_stop_minutes == 0, "настройки по умолчанию");
    manager.start(&id).await.unwrap();
    wait_running(&manager, &id).await;

    // Копия на работающем сервере.
    let info = manager.backup(&id, BackupKind::Manual).await.unwrap().expect("мир есть");
    println!("== копия {} ({} байт)", info.file, info.size_bytes);
    let zip = manager.store().backups_dir(&id).join(&info.file);
    let names = zip_names(&zip);
    assert!(names.contains(&"world/level.dat".to_string()), "{names:?}");
    assert!(names.iter().any(|n| n.ends_with(".mca")), "регионы мира в копии");
    assert!(!names.iter().any(|n| n.ends_with("session.lock")));
    let reply = manager.rcon(&id, "save-on").await.unwrap();
    println!("== save-on после копии: {reply}");
    assert!(reply.contains("already"), "автосохранение снова включено: {reply}");

    // Экспорт мира для одиночной игры, тоже на ходу.
    let out = tempfile::tempdir().unwrap();
    let exported = out.path().join("world.zip");
    manager.export_world(&id, exported.to_str().unwrap()).await.unwrap();
    let names = zip_names(&exported);
    let folder = format!("Бэкапы {loader:?}");
    assert!(names.contains(&format!("{folder}/level.dat")), "{names:?}");
    // Ад внутри мира: `DIM-1/` у старых версий, `dimensions/minecraft/the_nether/` у 26.x.
    // Регионы ада есть только у Paper: Vanilla и Fabric не создают их, пока туда никто не зашёл.
    assert!(
        names.iter().any(|n| n.contains("/DIM-1/") || n.contains("/the_nether/")),
        "ад внутри мира: {names:?}"
    );

    // Восстановление и импорт только на остановленном сервере.
    assert!(manager.restore_backup(&id, &info.file).await.is_err());

    // Автовыключение: «минута» в тесте длится секунду.
    manager.set_minute_for_tests(Duration::from_secs(1));
    manager
        .update(&id, hearth_lib::servers::ServerPatch { auto_stop_minutes: Some(5), ..Default::default() })
        .await
        .unwrap();
    let waiting = Instant::now();
    while manager.status(&id) != ServerStatus::Stopped {
        assert!(waiting.elapsed() < Duration::from_secs(90), "пустой сервер не выключился сам");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(manager.console(&id).iter().any(|l| l.text.contains("никого нет")), "в консоли объяснение");
    println!("== выключился сам за {:?}", waiting.elapsed());

    let game = manager.store().game_dir(&id);
    std::fs::write(game.join("world").join("marker.txt"), b"after").unwrap();
    let safety = manager.restore_backup(&id, &info.file).await.unwrap().expect("страховочная копия");
    assert!(!game.join("world/marker.txt").exists(), "мир вернулся к копии");
    assert!(zip_names(&manager.store().backups_dir(&id).join(&safety.file)).contains(&"world/marker.txt".to_string()));

    // Импорт мира в формате одиночной игры (это наш экспорт) и запуск с ним.
    manager.import_world(&id, exported.to_str().unwrap()).await.unwrap();
    assert!(game.join("world/level.dat").is_file());
    let kinds: Vec<BackupKind> = manager.backups(&id).unwrap().into_iter().map(|b| b.kind).collect();
    println!("== копии: {kinds:?}");
    assert!(kinds.contains(&BackupKind::BeforeImport) && kinds.contains(&BackupKind::BeforeRestore));

    manager
        .update(&id, hearth_lib::servers::ServerPatch { auto_stop_minutes: Some(0), ..Default::default() })
        .await
        .unwrap();
    manager.start(&id).await.unwrap();
    wait_running(&manager, &id).await;
    manager.stop(&id, STOP_TIMEOUT).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_backups_paper() {
    run_backups(Loader::Paper).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_backups_vanilla() {
    run_backups(Loader::Vanilla).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_backups_fabric() {
    run_backups(Loader::Fabric).await;
}

/// Плагины и моды с Modrinth на настоящем сервере: поставить, запустить,
/// убедиться, что загрузился, и удалить на остановленном.
async fn run_mods(loader: Loader, project: &str, marker: &str) {
    let shared = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("hearth-live");
    let servers = tempfile::tempdir().unwrap();
    let paths = Paths::new(shared.join("config"), shared.join("data"));
    let http = Http::new(paths.cache_dir()).unwrap();
    let manager = test_manager(&paths, &http, servers.path(), Arc::new(PrintSink::default()));
    let version = providers::list_versions(&http, loader, false).await.unwrap()[0].id.clone();
    println!("== моды {loader:?} {version}: {project}");
    let id = manager
        .create(CreateServerRequest {
            name: format!("Моды {loader:?}"),
            loader,
            game_version: version,
            memory_mb: 2048,
            port: Some(free_port()),
            game_mode: GameMode::Creative,
            difficulty: Difficulty::Peaceful,
            max_players: 3,
            seed: None,
            eula_accepted: true,
            pirate_mode: false,
            nick_protection: false,
        })
        .await
        .unwrap()
        .id;

    let found = manager.search_mods(&id, marker, 0).await.unwrap();
    assert!(found.hits.iter().any(|h| h.title.contains(marker)), "поиск находит {marker}");
    let files = manager.install_mod(&id, project).await.unwrap();
    println!("== поставлено: {files:?}");
    assert!(!files.is_empty());

    manager.start(&id).await.unwrap();
    wait_running(&manager, &id).await;
    assert!(
        manager.console(&id).iter().any(|l| l.text.contains(marker)),
        "в логе видно, что {marker} загрузился"
    );
    // Статистика (команды Paper для Purpur) работает.
    let started = Instant::now();
    while manager.get(&id).unwrap().stats.and_then(|s| s.tps).is_none() {
        assert!(started.elapsed() < Duration::from_secs(30), "TPS не пришёл");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let listed = manager.mods(&id).await.unwrap();
    println!("== список: {:?}", listed.items.iter().map(|m| (&m.title, &m.version)).collect::<Vec<_>>());
    assert!(listed.online && listed.items.iter().any(|m| m.title.contains(marker)));
    assert!(manager.remove_mod(&id, &files[0]).await.is_err(), "на работающем не удаляет");
    manager.stop(&id, STOP_TIMEOUT).await.unwrap();
    for file in &files {
        manager.remove_mod(&id, file).await.unwrap();
    }
    assert!(manager.mods(&id).await.unwrap().items.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_mods_purpur() {
    run_mods(Loader::Purpur, "luckperms", "LuckPerms").await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_mods_fabric() {
    run_mods(Loader::Fabric, "gvQqBUqZ", "Lithium").await;
}
