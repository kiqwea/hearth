# Hearth

Десктопное приложение для Windows: создаёт и запускает сервер Minecraft на вашем ПК в пару кликов. Техническое задание и принятые решения описаны в [SPEC.md](SPEC.md).

Сейчас готов этап 1: Vanilla, Paper и Fabric, автоустановка Java, запуск и корректная остановка, консоль, настройки сервера, несколько серверов, трей. Из этапа 2 готов доступ для друзей: сначала напрямую (Hearth открывает порт на роутере через UPnP), а если провайдер или роутер не позволяют, через playit.gg с постоянным адресом.

## Что нужно для сборки

- Rust (stable, MSVC) через rustup
- Visual Studio Build Tools 2022 с компонентом «Разработка классических приложений на C++»
- Node.js LTS
- WebView2 (в Windows 11 уже есть)

## Команды

```powershell
npm install
npm run tauri dev      # запуск в режиме разработки
npm run tauri build    # установщик NSIS в src-tauri/target/release/bundle/nsis
```

Только интерфейс в обычном браузере, на выдуманных данных (Rust не нужен, удобно для правок дизайна):

```powershell
npm run dev            # затем открыть http://localhost:1420
```

Тесты ядра (заодно обновляют TypeScript-типы в `src/bindings`):

```powershell
cd src-tauri
cargo test
```

Живые тесты: качают Java и ядра, запускают и останавливают настоящие серверы Vanilla, Paper и Fabric. Нужен интернет и несколько минут:

```powershell
cd src-tauri
cargo test --test live -- --ignored --test-threads=1 --nocapture
```

## Устройство

```
src-tauri/src/
  providers/   Vanilla (Mojang), Paper (Fill API v3), Fabric (meta): версии и файлы сервера
  java.rs      выбор и установка Eclipse Temurin (Adoptium API)
  servers/     server.json, server.properties, eula.txt
  process/     статусы, буфер консоли, аргументы запуска, Windows Job Object
  manager.rs   установка, запуск, корректный stop, kill, команды, доступ для друзей
  direct.rs    прямое подключение: проброс порта на роутере через UPnP
  tunnel/      playit.gg: API, ключ в диспетчере учётных данных, скачивание и запуск агента
  net.rs       HTTP-клиент: User-Agent, кэш API, скачивание с проверкой хэша
  app/         Tauri: команды для фронтенда, трей, закрытие в трей
src/
  bindings/    типы из Rust (генерируются, руками не править)
  lib/api.ts   обёртка над командами ядра
```

Данные приложения хранятся так:

- `%APPDATA%\Hearth\config.json`: настройки
- `%LOCALAPPDATA%\Hearth\`: Java, кэш, логи, агент playit, серверы по умолчанию
- Ключ агента playit: диспетчер учётных данных Windows, запись `Hearth/playit.gg`
