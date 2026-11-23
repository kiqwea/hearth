// Типизированная обёртка над командами Rust-ядра.
// Типы в ../bindings генерируются из Rust (ts-rs) командой `cargo test` в src-tauri.
//
// Если страница открыта в обычном браузере (npm run dev без Tauri), вместо ядра
// работает имитация из mock.ts: так интерфейс можно смотреть и править без сборки Rust.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { AppConfig } from "../bindings/AppConfig";
import type { BackupInfo } from "../bindings/BackupInfo";
import type { ModList } from "../bindings/ModList";
import type { ModSearch } from "../bindings/ModSearch";
import type { SavedWorld } from "../bindings/SavedWorld";
import type { ConsoleLine } from "../bindings/ConsoleLine";
import type { CreateServerRequest } from "../bindings/CreateServerRequest";
import type { DirectAttempt } from "../bindings/DirectAttempt";
import type { Event } from "../bindings/Event";
import type { GameVersion } from "../bindings/GameVersion";
import type { InstalledJava } from "../bindings/InstalledJava";
import type { Loader } from "../bindings/Loader";
import type { NetworkInfo } from "../bindings/NetworkInfo";
import type { PirateResult } from "../bindings/PirateResult";
import type { PlayerAction } from "../bindings/PlayerAction";
import type { PlayerInfo } from "../bindings/PlayerInfo";
import type { PlayitStatus } from "../bindings/PlayitStatus";
import type { PropertyEntry } from "../bindings/PropertyEntry";
import type { ServerMeta } from "../bindings/ServerMeta";
import type { ServerPatch } from "../bindings/ServerPatch";
import type { ServerSummary } from "../bindings/ServerSummary";
import type { SystemInfo } from "../bindings/SystemInfo";
import { createMockApi } from "./mock";

/** Ошибка из Rust: `kind` для логики интерфейса, `message` для показа пользователю. */
export interface HearthError {
  kind: string;
  message: string;
}

export function isHearthError(e: unknown): e is HearthError {
  return typeof e === "object" && e !== null && "kind" in e && "message" in e;
}

export function errorMessage(e: unknown): string {
  if (isHearthError(e)) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}

export interface HearthApi {
  systemInfo(): Promise<SystemInfo>;
  networkInfo(): Promise<NetworkInfo>;
  getConfig(): Promise<AppConfig>;
  setConfig(config: AppConfig): Promise<AppConfig>;
  listVersions(loader: Loader, includeUnstable: boolean): Promise<GameVersion[]>;
  listJava(): Promise<InstalledJava[]>;
  listServers(): Promise<ServerSummary[]>;
  getServer(id: string): Promise<ServerSummary>;
  createServer(request: CreateServerRequest): Promise<ServerMeta>;
  updateServer(id: string, patch: ServerPatch): Promise<ServerMeta>;
  /** `deleteTunnel`: удалить и адрес сервера в аккаунте playit. */
  deleteServer(id: string, deleteTunnel: boolean): Promise<void>;
  installServer(id: string): Promise<ServerMeta>;
  start(id: string): Promise<void>;
  stop(id: string): Promise<void>;
  kill(id: string): Promise<void>;
  restart(id: string): Promise<void>;
  sendCommand(id: string, command: string): Promise<void>;
  console(id: string): Promise<ConsoleLine[]>;
  getProperties(id: string): Promise<PropertyEntry[]>;
  setProperties(id: string, entries: PropertyEntry[]): Promise<void>;
  openFolder(id: string): Promise<void>;
  stopAll(): Promise<void>;

  // Доступ для друзей через playit.gg
  playitStatus(): Promise<PlayitStatus>;
  /** Начинает подключение к аккаунту playit; возвращает ссылку для браузера. */
  playitStartClaim(): Promise<string>;
  playitCancelClaim(): Promise<void>;
  playitUnlink(): Promise<void>;
  playitRetry(): Promise<void>;
  playitDashboardUrl(): Promise<string>;
  enableFriendAccess(id: string): Promise<ServerMeta>;
  /** Сначала без посредников: проброс порта на роутере (UPnP). */
  tryDirectAccess(id: string): Promise<DirectAttempt>;
  /** Пиратский режим: бэкап, плагин авторизации, перенос вещей игроков. Сервер должен быть остановлен. */
  setPirateMode(id: string, enable: boolean, protect: boolean): Promise<PirateResult>;
  /** Защита ников паролем (AuthMe/EasyAuth) на сервере в пиратском режиме. */
  setNickProtection(id: string, on: boolean): Promise<ServerMeta>;

  // Игроки
  getPlayers(id: string): Promise<PlayerInfo[]>;
  /** Кик, бан, оператор, белый список. Возвращает ответ сервера. */
  playerAction(id: string, action: PlayerAction, name: string, reason: string | null): Promise<string>;
  disableFriendAccess(id: string): Promise<ServerMeta>;

  // Миры и резервные копии
  listBackups(id: string): Promise<BackupInfo[]>;
  backupInProgress(id: string): Promise<boolean>;
  /** Копия мира сейчас (и на работающем сервере). `null`, если мира ещё нет. */
  createBackup(id: string): Promise<BackupInfo | null>;
  deleteBackup(id: string, file: string): Promise<void>;
  /** Сервер должен быть остановлен. Возвращает страховочную копию прежнего мира. */
  restoreBackup(id: string, file: string): Promise<BackupInfo | null>;
  exportBackup(id: string, file: string, dest: string): Promise<void>;
  exportWorld(id: string, dest: string): Promise<void>;
  listSavedWorlds(): Promise<SavedWorld[]>;
  /** Папка мира или zip. Сервер должен быть остановлен. */
  importWorld(id: string, source: string): Promise<BackupInfo | null>;
  openBackupsFolder(id: string): Promise<void>;
  // Плагины и моды (Modrinth)
  listMods(id: string): Promise<ModList>;
  searchMods(id: string, query: string, offset: number): Promise<ModSearch>;
  /** Ставит проект с зависимостями. Возвращает поставленные файлы. */
  installMod(id: string, project: string): Promise<string[]>;
  /** Сервер должен быть остановлен. */
  updateMods(id: string): Promise<string[]>;
  removeMod(id: string, file: string): Promise<void>;
  openModsFolder(id: string): Promise<void>;
  /** Системные окна выбора файла. `null`, если пользователь передумал. */
  pickZip(): Promise<string | null>;
  pickFolder(): Promise<string | null>;
  pickSaveZip(defaultName: string): Promise<string | null>;

  onEvent(handler: (event: Event) => void): Promise<() => void>;
  /** Есть ли новая версия Hearth. `null`, если установлена последняя. */
  checkUpdate(): Promise<AvailableUpdate | null>;
}

export interface AvailableUpdate {
  version: string;
  currentVersion: string;
  notes: string | null;
  date: string | null;
  /** Скачивает, проверяет подпись и ставит обновление; Hearth перезапустится сам. */
  install(onProgress: (downloaded: number, total: number | null) => void): Promise<void>;
}

async function tauriCheckUpdate(): Promise<AvailableUpdate | null> {
  const { check } = await import("@tauri-apps/plugin-updater");
  const update = await check();
  if (!update) return null;
  return {
    version: update.version,
    currentVersion: update.currentVersion,
    notes: update.body ?? null,
    date: update.date ?? null,
    async install(onProgress) {
      let total: number | null = null;
      let downloaded = 0;
      await update.downloadAndInstall((e) => {
        if (e.event === "Started") total = e.data.contentLength ?? null;
        if (e.event === "Progress") downloaded += e.data.chunkLength;
        onProgress(downloaded, total);
      });
      // На Windows установщик сам перезапускает Hearth; relaunch — на случай других ОС.
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    },
  };
}

const tauriApi: HearthApi = {
  systemInfo: () => invoke("get_system_info"),
  networkInfo: () => invoke("get_network_info"),
  getConfig: () => invoke("get_config"),
  setConfig: (config) => invoke("set_config", { config }),

  listVersions: (loader, includeUnstable) => invoke("list_versions", { loader, includeUnstable }),
  listJava: () => invoke("list_java"),

  listServers: () => invoke("list_servers"),
  getServer: (id) => invoke("get_server", { id }),
  createServer: (request) => invoke("create_server", { request }),
  updateServer: (id, patch) => invoke("update_server", { id, patch }),
  deleteServer: (id, deleteTunnel) => invoke("delete_server", { id, deleteTunnel }),
  installServer: (id) => invoke("install_server", { id }),

  start: (id) => invoke("start_server", { id }),
  stop: (id) => invoke("stop_server", { id }),
  kill: (id) => invoke("kill_server", { id }),
  restart: (id) => invoke("restart_server", { id }),
  sendCommand: (id, command) => invoke("send_command", { id, command }),
  console: (id) => invoke("get_console", { id }),

  getProperties: (id) => invoke("get_properties", { id }),
  setProperties: (id, entries) => invoke("set_properties", { id, entries }),
  openFolder: (id) => invoke("open_server_folder", { id }),
  stopAll: () => invoke("stop_all_servers"),

  playitStatus: () => invoke("get_playit_status"),
  playitStartClaim: () => invoke("playit_start_claim"),
  playitCancelClaim: () => invoke("playit_cancel_claim"),
  playitUnlink: () => invoke("playit_unlink"),
  playitRetry: () => invoke("playit_retry"),
  playitDashboardUrl: () => invoke("playit_dashboard_url"),
  enableFriendAccess: (id) => invoke("enable_friend_access", { id }),
  tryDirectAccess: (id) => invoke("try_direct_access", { id }),
  setPirateMode: (id, enable, protect) => invoke("set_pirate_mode", { id, enable, protect }),
  setNickProtection: (id, on) => invoke("set_nick_protection", { id, on }),
  getPlayers: (id) => invoke("get_players", { id }),
  playerAction: (id, action, name, reason) => invoke("player_action", { id, action, name, reason }),
  disableFriendAccess: (id) => invoke("disable_friend_access", { id }),

  listBackups: (id) => invoke("list_backups", { id }),
  backupInProgress: (id) => invoke("backup_in_progress", { id }),
  createBackup: (id) => invoke("create_backup", { id }),
  deleteBackup: (id, file) => invoke("delete_backup", { id, file }),
  restoreBackup: (id, file) => invoke("restore_backup", { id, file }),
  exportBackup: (id, file, dest) => invoke("export_backup", { id, file, dest }),
  exportWorld: (id, dest) => invoke("export_world", { id, dest }),
  listSavedWorlds: () => invoke("list_saved_worlds"),
  importWorld: (id, source) => invoke("import_world", { id, source }),
  openBackupsFolder: (id) => invoke("open_backups_folder", { id }),
  listMods: (id) => invoke("list_mods", { id }),
  searchMods: (id, query, offset) => invoke("search_mods", { id, query, offset }),
  installMod: (id, project) => invoke("install_mod", { id, project }),
  updateMods: (id) => invoke("update_mods", { id }),
  removeMod: (id, file) => invoke("remove_mod", { id, file }),
  openModsFolder: (id) => invoke("open_mods_folder", { id }),
  pickZip: async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, filters: [{ name: "Zip-архив", extensions: ["zip"] }] });
    return typeof path === "string" ? path : null;
  },
  pickFolder: async () => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ directory: true, multiple: false });
    return typeof path === "string" ? path : null;
  },
  pickSaveZip: async (defaultName) => {
    const { save } = await import("@tauri-apps/plugin-dialog");
    return save({ defaultPath: defaultName, filters: [{ name: "Zip-архив", extensions: ["zip"] }] });
  },

  onEvent: (handler) => listen<Event>("hearth://event", (e) => handler(e.payload)),
  checkUpdate: tauriCheckUpdate,
};

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const api: HearthApi = inTauri ? tauriApi : createMockApi();

/** Открывает ссылку в браузере пользователя, а не внутри окна Hearth. */
export async function openExternal(url: string): Promise<void> {
  if (inTauri) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } else {
    window.open(url, "_blank", "noopener");
  }
}
