// Имитация ядра для просмотра интерфейса в обычном браузере (`npm run dev`).
// В приложении не используется: там работает настоящий Rust.

import type { ConsoleLine } from "../bindings/ConsoleLine";
import type { ConsoleStream } from "../bindings/ConsoleStream";
import type { Event } from "../bindings/Event";
import type { GameVersion } from "../bindings/GameVersion";
import type { DirectInfo } from "../bindings/DirectInfo";
import type { InstallProgress } from "../bindings/InstallProgress";
import type { PlayitAgentState } from "../bindings/PlayitAgentState";
import type { PlayitStatus } from "../bindings/PlayitStatus";
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
    playit: null,
    directAccess: false,
    pirateMode: false,
    authPlugin: null,
    authLangSet: false,
    ...partial,
  };
}

export function createMockApi(): HearthApi {
  const handlers = new Set<(e: Event) => void>();
  const emit = (e: Event) => handlers.forEach((h) => h(e));

  const servers = new Map<
    string,
    { meta: ServerMeta; status: ServerStatus; props: Record<string, string>; direct?: DirectInfo | null }
  >();
  // Прямое подключение: у «Модового» роутер открывает порт, у остальных — нет (как при CGNAT).
  const directWorks = (id: string) => id.startsWith("modded");
  const setDirect = (id: string, info: DirectInfo) => {
    servers.get(id)!.direct = info;
    emit({ type: "direct", data: { serverId: id, info } });
  };
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

  // playit.gg: подключение аккаунта, агент и туннели.
  const playit: PlayitStatus = {
    linked: false,
    claimUrl: null,
    claimError: null,
    agent: "off",
    agentError: null,
    tunnels: [],
    synced: true,
    guest: true,
    notices: [],
  };
  const playitUsers = new Set<string>();
  const emitPlayit = () => emit({ type: "playit", data: structuredClone(playit) });
  const setAgent = (agent: PlayitAgentState) => {
    playit.agent = agent;
    emitPlayit();
  };
  const acquireAgent = async (id: string) => {
    playitUsers.add(id);
    if (playit.agent !== "off" && playit.agent !== "error") return;
    setAgent("downloading");
    await wait(1200);
    setAgent("starting");
    await wait(1000);
    if (playitUsers.size > 0) setAgent("online");
  };
  const createTunnel = async (id: string): Promise<ServerMeta> => {
    const s = servers.get(id)!;
    await wait(800);
    const tunnelId = `tun-${Math.random().toString(16).slice(2, 10)}`;
    playit.tunnels.push({ id: tunnelId, address: null, pending: "Выделяю адрес", disabled: null });
    s.meta.playit = { tunnelId, localPort: s.meta.port, createdAt: new Date().toISOString() };
    emitPlayit();
    setTimeout(() => {
      const t = playit.tunnels.find((t) => t.id === tunnelId);
      if (!t) return;
      t.address = `${s.meta.id.split("-")[0]}-friends.gl.joinmc.link`;
      t.pending = null;
      emitPlayit();
    }, 2500);
    if (s.status.state === "running") void acquireAgent(id);
    return { ...s.meta };
  };
  const releaseAgent = (id: string) => {
    playitUsers.delete(id);
    if (playitUsers.size === 0) setAgent("off");
  };

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
    if (s.meta.playit) void acquireAgent(id);
    if (s.meta.directAccess) {
      setDirect(id, { state: "opening", address: s.direct?.address ?? null, error: null });
      await wait(1200);
      setDirect(id, { state: "open", address: "92.208.220.160", error: null });
    } else if (!s.meta.playit) {
      // Как в ядре: доступ для друзей настраивается сам при запуске.
      setDirect(id, { state: "opening", address: null, error: null });
      await wait(1500);
      if (directWorks(id)) {
        s.meta.directAccess = true;
        setDirect(id, { state: "open", address: "92.208.220.160", error: null });
      } else {
        setDirect(id, {
          state: "failed",
          address: null,
          error: "роутер не разрешает автоматически открыть порт (UPnP выключен или не поддерживается)",
        });
        if (playit.linked) await createTunnel(id);
      }
    }
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
    return { meta: { ...s.meta }, status: s.status, direct: s.direct ?? null };
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
    deleteServer: async (id, deleteTunnel) => {
      const tunnelId = servers.get(id)?.meta.playit?.tunnelId;
      if (deleteTunnel && tunnelId) playit.tunnels = playit.tunnels.filter((t) => t.id !== tunnelId);
      servers.delete(id);
      consoles.delete(id);
      releaseAgent(id);
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
      releaseAgent(id);
      const s = servers.get(id)!;
      if (s.meta.directAccess) setDirect(id, { state: "off", address: s.direct?.address ?? null, error: null });
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

    playitStatus: async () => structuredClone(playit),
    playitStartClaim: async () => {
      if (!playit.claimUrl) {
        playit.claimUrl = "https://playit.gg/claim/0a1b2c3d4e";
        playit.claimError = null;
        emitPlayit();
        // Будто пользователь вошёл в playit и подтвердил подключение.
        setTimeout(() => {
          if (!playit.claimUrl) return;
          playit.claimUrl = null;
          playit.linked = true;
          emitPlayit();
        }, 5000);
      }
      return playit.claimUrl;
    },
    playitCancelClaim: async () => {
      playit.claimUrl = null;
      emitPlayit();
    },
    playitUnlink: async () => {
      playitUsers.clear();
      playit.linked = false;
      playit.agent = "off";
      emitPlayit();
    },
    playitRetry: async () => {},
    playitDashboardUrl: async () => "https://playit.gg/account/tunnels",
    enableFriendAccess: async (id) => {
      if (!playit.linked) throw { kind: "playitNotLinked", message: "Сначала подключите playit.gg" };
      return createTunnel(id);
    },
    setPirateMode: async (id, enable, allowWithoutAuth) => {
      const s = servers.get(id)!;
      const plugin = s.meta.loader === "paper" ? "AuthMe Reloaded" : s.meta.loader === "fabric" ? "EasyAuth" : null;
      if (enable && !plugin && !allowWithoutAuth) {
        throw { kind: "authUnavailable", message: "Для Vanilla нет плагинов авторизации" };
      }
      await wait(1800);
      s.meta.pirateMode = enable;
      s.meta.authPlugin = enable && plugin ? `${plugin.split(" ")[0]}.jar` : null;
      s.props["online-mode"] = String(!enable);
      return {
        meta: { ...s.meta },
        backup: "C:\\Users\\…\\backups\\2026-11-22_pirate-on.zip",
        migrated: 3,
        auth: enable ? plugin : null,
      };
    },
    tryDirectAccess: async (id) => {
      const s = servers.get(id)!;
      await wait(1500);
      if (!directWorks(id)) {
        return {
          meta: { ...s.meta },
          address: null,
          reason: "роутер не разрешает автоматически открыть порт (UPnP выключен или не поддерживается)",
        };
      }
      s.meta.directAccess = true;
      const running = s.status.state === "running";
      setDirect(id, { state: running ? "open" : "off", address: "92.208.220.160", error: null });
      return { meta: { ...s.meta }, address: "92.208.220.160", reason: null };
    },
    disableFriendAccess: async (id) => {
      const s = servers.get(id)!;
      await wait(500);
      if (s.meta.directAccess) {
        s.meta.directAccess = false;
        setDirect(id, { state: "off", address: null, error: null });
        return { ...s.meta };
      }
      playit.tunnels = playit.tunnels.filter((t) => t.id !== s.meta.playit?.tunnelId);
      s.meta.playit = null;
      releaseAgent(id);
      emitPlayit();
      return { ...s.meta };
    },
    stopAll: async () => {
      for (const [id, s] of servers) {
        if (s.status.state === "running" || s.status.state === "starting") {
          setStatus(id, { state: "stopping" });
          await wait(600);
          setStatus(id, { state: "stopped" });
        }
      }
    },
    checkUpdate: async () => ({
      version: "0.2.0",
      currentVersion: "0.1.0",
      notes: "- Доступ для друзей через интернет (playit.gg)\n- Вкладка «Игроки»: кик, бан, оп и белый список",
      date: null,
      install: async (onProgress) => {
        const total = 3_300_000;
        for (let d = 0; d <= total; d += 300_000) {
          onProgress(d, total);
          await wait(150);
        }
        window.location.reload();
      },
    }),
    onEvent: async (handler) => {
      handlers.add(handler);
      return () => handlers.delete(handler);
    },
  };
}
