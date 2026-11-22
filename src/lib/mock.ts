// Имитация ядра для просмотра интерфейса в обычном браузере (`npm run dev`).
// В приложении не используется: там работает настоящий Rust.

import type { ConsoleLine } from "../bindings/ConsoleLine";
import type { ConsoleStream } from "../bindings/ConsoleStream";
import type { Event } from "../bindings/Event";
import type { GameVersion } from "../bindings/GameVersion";
import type { InstallProgress } from "../bindings/InstallProgress";
import type { ServerMeta } from "../bindings/ServerMeta";
import type { ServerStatus } from "../bindings/ServerStatus";
import type { HearthApi } from "./api";

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

const DEFAULT_PROPS: Record<string, string> = {
  "allow-flight": "false",
  difficulty: "normal",
  "enable-rcon": "false",
  "enforce-whitelist": "false",
  "force-gamemode": "false",
  gamemode: "survival",
  "generate-structures": "true",
  hardcore: "false",
  "level-name": "world",
  "level-seed": "",
  "level-type": "minecraft:normal",
  "max-players": "10",
  motd: "Сервер Димы и друзей",
  "online-mode": "true",
  pvp: "true",
  "server-port": "25565",
  "simulation-distance": "10",
  "spawn-protection": "16",
  "view-distance": "10",
  "white-list": "false",
};

function meta(partial: Partial<ServerMeta> & Pick<ServerMeta, "id" | "name" | "loader" | "gameVersion">): ServerMeta {
  return {
    schema: 1,
    edition: "java",
    loaderVersion: null,
    jar: "server.jar",
    javaMajor: 25,
    memoryMb: 4096,
    port: 25565,
    jvmFlags: [],
    installed: true,
    eulaAcceptedAt: new Date().toISOString(),
    createdAt: new Date().toISOString(),
    ...partial,
  };
}

export function createMockApi(): HearthApi {
  const handlers = new Set<(e: Event) => void>();
  const emit = (e: Event) => handlers.forEach((h) => h(e));

  const servers = new Map<string, { meta: ServerMeta; status: ServerStatus; props: Record<string, string> }>();
  const consoles = new Map<string, ConsoleLine[]>();
  let seq = 0;

  const add = (m: ServerMeta, status: ServerStatus) => {
    servers.set(m.id, { meta: m, status, props: { ...DEFAULT_PROPS, motd: m.name, "server-port": String(m.port) } });
    consoles.set(m.id, []);
  };
  add(meta({ id: "survival-a1b2c3", name: "Выживание с друзьями", loader: "paper", gameVersion: "26.3", loaderVersion: "258", jar: "paper.jar" }), { state: "running" });
  add(meta({ id: "modded-d4e5f6", name: "Модовый", loader: "fabric", gameVersion: "26.3", loaderVersion: "0.20.1", memoryMb: 6144 }), { state: "stopped" });
  add(meta({ id: "creative-0a1b2c", name: "Креатив", loader: "vanilla", gameVersion: "1.21.11", javaMajor: 21, memoryMb: 2048 }), { state: "stopped" });

  const line = (id: string, stream: ConsoleStream, text: string) => {
    const l: ConsoleLine = { seq: seq++, stream, text };
    consoles.get(id)!.push(l);
    emit({ type: "console", data: { serverId: id, line: l } });
  };
  const setStatus = (id: string, status: ServerStatus) => {
    servers.get(id)!.status = status;
    emit({ type: "status", data: { serverId: id, status } });
  };
  const progress = (id: string, p: InstallProgress) => emit({ type: "install", data: { serverId: id, progress: p } });

  [
    "Запуск Paper 26.3 (Java 25, память 4096 МБ, порт 25565)",
  ].forEach((t) => line("survival-a1b2c3", "system", t));
  [
    "[17:50:02 INFO]: Starting minecraft server version 26.3",
    "[17:50:02 INFO]: Loading properties",
    "[17:50:04 INFO]: Preparing level \"world\"",
    "[17:50:08 INFO]: \u001b[32mDone (4.610s)! For help, type \"help\"\u001b[0m",
    "[18:02:11 INFO]: Sanya2010 joined the game",
    "[18:02:40 INFO]: <Sanya2010> го в шахту",
    "[18:06:19 WARN]: \u001b[33mCan't keep up! Is the server overloaded? Running 2104ms or 42 ticks behind\u001b[0m",
    "[18:10:03 INFO]: lenka_mur joined the game",
    "[18:12:57 INFO]: Dima_K has made the advancement \u001b[32m[Diamonds!]\u001b[0m",
  ].forEach((t) => line("survival-a1b2c3", "stdout", t));

  const boot = async (id: string) => {
    const s = servers.get(id)!;
    if (!s.meta.installed) {
      setStatus(id, { state: "installing" });
      progress(id, { stage: "resolving", message: `Получаю данные о ${s.meta.loader} ${s.meta.gameVersion}`, downloaded: null, total: null });
      await wait(900);
      const javaTotal = 58_000_000;
      for (let d = 0; d <= javaTotal; d += 4_000_000) {
        progress(id, { stage: "java", message: "Подготавливаю Java 25", downloaded: d, total: javaTotal });
        await wait(120);
      }
      const coreTotal = 53_000_000;
      for (let d = 0; d <= coreTotal; d += 6_000_000) {
        progress(id, { stage: "server", message: "Скачиваю paper.jar", downloaded: d, total: coreTotal });
        await wait(120);
      }
      s.meta.installed = true;
      progress(id, { stage: "done", message: "Сервер установлен", downloaded: null, total: null });
      setStatus(id, { state: "stopped" });
    }
    consoles.set(id, consoles.get(id)!.slice(0, 0));
    line(id, "system", `Запуск ${s.meta.loader} ${s.meta.gameVersion} (Java 25, память ${s.meta.memoryMb} МБ, порт ${s.meta.port})`);
    setStatus(id, { state: "starting" });
    await wait(700);
    line(id, "stdout", `[18:20:01 INFO]: Starting minecraft server version ${s.meta.gameVersion}`);
    await wait(700);
    line(id, "stdout", "[18:20:03 INFO]: Preparing level \"world\"");
    await wait(900);
    line(id, "stdout", "[18:20:05 INFO]: Done (3.912s)! For help, type \"help\"");
    setStatus(id, { state: "running" });
  };

  const versions: GameVersion[] = [
    { id: "26.4-snapshot-10", stable: false, releaseTime: null },
    { id: "26.3", stable: true, releaseTime: null },
    { id: "26.2", stable: true, releaseTime: null },
    { id: "26.1.2", stable: true, releaseTime: null },
    { id: "1.21.11", stable: true, releaseTime: null },
    { id: "1.20.6", stable: true, releaseTime: null },
  ];

  const summary = (id: string) => {
    const s = servers.get(id);
    if (!s) throw { kind: "serverNotFound", message: "Сервер не найден" };
    return { meta: { ...s.meta }, status: s.status };
  };

  return {
    systemInfo: async () => ({ totalMemoryMb: 16_384, recommendedMemoryMb: 4096, appVersion: "0.1.0-preview" }),
    networkInfo: async () => ({ lanIp: "192.168.1.5" }),
    getConfig: async () => ({ serversDir: null, closeToTray: true, language: "ru" }),
    setConfig: async (c) => c,
    listVersions: async (_loader, unstable) => {
      await wait(400);
      return versions.filter((v) => unstable || v.stable);
    },
    listJava: async () => [{ major: 25, release: "jdk-25.0.5.1+2", image: "jre" }],
    listServers: async () => [...servers.keys()].map(summary),
    getServer: async (id) => summary(id),
    createServer: async (req) => {
      const id = `server-${Math.random().toString(16).slice(2, 8)}`;
      add(
        meta({ id, name: req.name, loader: req.loader, gameVersion: req.gameVersion, memoryMb: req.memoryMb, port: req.port ?? 25565, installed: false, jar: null }),
        { state: "stopped" },
      );
      return summary(id).meta;
    },
    updateServer: async (id, patch) => {
      const s = servers.get(id)!;
      if (patch.name) s.meta.name = patch.name;
      if (patch.memoryMb) s.meta.memoryMb = patch.memoryMb;
      return { ...s.meta };
    },
    deleteServer: async (id) => {
      servers.delete(id);
      consoles.delete(id);
    },
    installServer: async (id) => summary(id).meta,
    start: async (id) => {
      void boot(id);
      await wait(50);
    },
    stop: async (id) => {
      setStatus(id, { state: "stopping" });
      line(id, "input", "stop");
      await wait(600);
      line(id, "stdout", "[18:30:00 INFO]: Stopping the server");
      line(id, "stdout", "[18:30:00 INFO]: Saving worlds");
      await wait(900);
      line(id, "system", "Сервер остановлен");
      setStatus(id, { state: "stopped" });
    },
    kill: async (id) => {
      line(id, "system", "Принудительное завершение сервера");
      setStatus(id, { state: "stopped" });
    },
    restart: async (id) => {
      setStatus(id, { state: "stopping" });
      await wait(800);
      await boot(id);
    },
    sendCommand: async (id, command) => {
      line(id, "input", command);
      await wait(150);
      if (command.startsWith("say ")) line(id, "stdout", `[18:31:00 INFO]: [Server] ${command.slice(4)}`);
      else if (command === "list") line(id, "stdout", "[18:31:00 INFO]: There are 3 of a max of 10 players online: Dima_K, Sanya2010, lenka_mur");
      else line(id, "stdout", `[18:31:00 INFO]: Unknown or incomplete command, see below for error`);
    },
    console: async (id) => [...(consoles.get(id) ?? [])],
    getProperties: async (id) =>
      Object.entries(servers.get(id)!.props).map(([key, value]) => ({ key, value })),
    setProperties: async (id, entries) => {
      const s = servers.get(id)!;
      for (const e of entries) {
        s.props[e.key] = e.value;
        if (e.key === "server-port") s.meta.port = Number(e.value);
      }
    },
    openFolder: async () => {},
    onEvent: async (handler) => {
      handlers.add(handler);
      return () => handlers.delete(handler);
    },
  };
}
