// Состояние интерфейса: серверы, их статусы, консоли, прогресс установки, экран.
// Ядро присылает события, store складывает их сюда, компоненты читают.

import { create } from "zustand";

import type { AppConfig } from "../bindings/AppConfig";
import type { ConsoleLine } from "../bindings/ConsoleLine";
import type { Event } from "../bindings/Event";
import type { InstallProgress } from "../bindings/InstallProgress";
import type { NetworkInfo } from "../bindings/NetworkInfo";
import type { PlayitStatus } from "../bindings/PlayitStatus";
import type { ServerStatus } from "../bindings/ServerStatus";
import type { ServerSummary } from "../bindings/ServerSummary";
import type { SystemInfo } from "../bindings/SystemInfo";
import { t } from "../i18n";
import { api, errorMessage, openExternal, type AvailableUpdate } from "./api";

/** Сколько строк консоли держим в интерфейсе на сервер (столько же хранит ядро). */
const CONSOLE_LIMIT = 5000;
/** Как часто проверять обновления Hearth, пока приложение открыто. */
const UPDATE_CHECK_INTERVAL = 6 * 60 * 60 * 1000;

export type ServerTab = "overview" | "console" | "players" | "settings" | "backups";

export type View =
  | { kind: "loading" }
  | { kind: "welcome" }
  | { kind: "wizard" }
  | { kind: "server"; id: string; tab: ServerTab };

export interface Toast {
  id: number;
  kind: "error" | "info" | "success";
  text: string;
}

interface HearthState {
  view: View;
  system: SystemInfo | null;
  network: NetworkInfo | null;
  config: AppConfig | null;
  servers: Record<string, ServerSummary>;
  consoles: Record<string, ConsoleLine[]>;
  install: Record<string, InstallProgress | undefined>;
  /** Когда сервер перешёл в «Работает» (для счётчика времени работы). */
  runningSince: Record<string, number>;
  toasts: Toast[];
  appSettingsOpen: boolean;
  /** Новая версия Hearth, если вышла. */
  update: AvailableUpdate | null;
  updateDialogOpen: boolean;
  /** Состояние playit.gg: подключение аккаунта, агент, адреса туннелей. */
  playit: PlayitStatus | null;
  /** Сервер, которому включим доступ для друзей, как только playit подключится. */
  pendingAccess: string | null;
  /** Серверы, для которых сейчас создаётся или удаляется адрес. */
  accessBusy: Record<string, boolean>;
  /** Сервер, для которого сейчас проверяется прямое подключение. */
  directTrying: Record<string, boolean>;
  /** Почему напрямую не получилось (показываем рядом с предложением playit). */
  directReason: Record<string, string>;
  /** Сейчас делается копия мира. */
  backupBusy: Record<string, boolean>;
  /** Растёт, когда список копий мог измениться: вкладка «Бэкапы» перечитывает его. */
  backupsRev: Record<string, number>;

  /**
   * «Включить доступ для друзей»: сначала напрямую (порт на роутере), если не
   * получилось — playit, при необходимости с подключением аккаунта в браузере.
   */
  connectFriendAccess(id: string, options?: { skipDirect?: boolean }): Promise<void>;
  /** Перейти с прямого подключения на playit (например, если друзья не могут зайти). */
  switchToPlayit(id: string): Promise<void>;
  enableFriendAccess(id: string): Promise<void>;
  disableFriendAccess(id: string): Promise<void>;
  cancelPlayitClaim(): Promise<void>;

  /** Проверка обновлений. `manual` — по кнопке: тогда сообщаем и об ошибке, и о том, что обновлений нет. */
  checkUpdates(manual?: boolean): Promise<void>;
  setUpdateDialogOpen(open: boolean): void;

  init(): Promise<void>;
  refreshServers(): Promise<void>;
  loadConsole(id: string): Promise<void>;
  navigate(view: View): void;
  openServer(id: string, tab?: ServerTab): void;
  setConfig(config: AppConfig): void;
  setAppSettingsOpen(open: boolean): void;
  toast(kind: Toast["kind"], text: string): void;
  toastError(e: unknown): void;
  dismissToast(id: number): void;
  removeServer(id: string): void;
}

let initialized = false;
let toastSeq = 0;

export const useHearth = create<HearthState>((set, get) => ({
  view: { kind: "loading" },
  system: null,
  network: null,
  config: null,
  servers: {},
  consoles: {},
  install: {},
  runningSince: {},
  toasts: [],
  appSettingsOpen: false,
  update: null,
  updateDialogOpen: false,
  playit: null,
  pendingAccess: null,
  accessBusy: {},
  directTrying: {},
  directReason: {},
  backupBusy: {},
  backupsRev: {},

  async connectFriendAccess(id, options) {
    const meta = get().servers[id]?.meta;
    if (!options?.skipDirect && meta && !meta.playit && !meta.directAccess) {
      set((st) => ({ directTrying: { ...st.directTrying, [id]: true } }));
      try {
        const attempt = await api.tryDirectAccess(id);
        setMeta(attempt.meta);
        if (attempt.address) {
          get().toast("success", t.access.enabledDirect);
          return;
        }
        if (attempt.reason) set((st) => ({ directReason: { ...st.directReason, [id]: attempt.reason! } }));
      } catch (e) {
        get().toastError(e);
        return;
      } finally {
        set((st) => ({ directTrying: { ...st.directTrying, [id]: false } }));
      }
    }
    const playit = get().playit;
    if (playit?.linked) return get().enableFriendAccess(id);
    set({ pendingAccess: id });
    try {
      const url = await api.playitStartClaim();
      await openExternal(url);
    } catch (e) {
      set({ pendingAccess: null });
      get().toastError(e);
    }
  },

  async switchToPlayit(id) {
    await get().disableFriendAccess(id);
    if (!get().servers[id]?.meta.directAccess) await get().connectFriendAccess(id, { skipDirect: true });
  },

  async enableFriendAccess(id) {
    set((st) => ({ accessBusy: { ...st.accessBusy, [id]: true } }));
    try {
      const meta = await api.enableFriendAccess(id);
      setMeta(meta);
      get().toast("success", t.access.enabled);
    } catch (e) {
      get().toastError(e);
    } finally {
      set((st) => ({ accessBusy: { ...st.accessBusy, [id]: false } }));
    }
  },

  async disableFriendAccess(id) {
    set((st) => ({ accessBusy: { ...st.accessBusy, [id]: true } }));
    try {
      setMeta(await api.disableFriendAccess(id));
    } catch (e) {
      get().toastError(e);
    } finally {
      set((st) => ({ accessBusy: { ...st.accessBusy, [id]: false } }));
    }
  },

  async cancelPlayitClaim() {
    set({ pendingAccess: null });
    try {
      await api.playitCancelClaim();
    } catch (e) {
      get().toastError(e);
    }
  },

  async checkUpdates(manual = false) {
    try {
      const update = await api.checkUpdate();
      set({ update });
      if (manual && !update) get().toast("success", t.update.upToDate);
    } catch (e) {
      // Без интернета или до первого релиза проверка просто не удаётся: молчим.
      if (manual) get().toast("error", `${t.update.checkFailed}: ${errorMessage(e)}`);
      else console.warn("Проверка обновлений не удалась", e);
    }
  },

  setUpdateDialogOpen(open) {
    set({ updateDialogOpen: open });
  },

  async init() {
    if (initialized) return;
    initialized = true;
    await api.onEvent(handleEvent);
    void get().checkUpdates();
    setInterval(() => void get().checkUpdates(), UPDATE_CHECK_INTERVAL);
    try {
      const [system, network, config, list] = await Promise.all([
        api.systemInfo(),
        api.networkInfo(),
        api.getConfig(),
        api.listServers(),
      ]);
      const servers = Object.fromEntries(list.map((s) => [s.meta.id, s]));
      const runningSince: Record<string, number> = {};
      for (const s of list) if (s.status.state === "running") runningSince[s.meta.id] = Date.now();
      set({
        system,
        network,
        config,
        servers,
        runningSince,
        view: list.length > 0 ? { kind: "server", id: list[0].meta.id, tab: "overview" } : { kind: "welcome" },
      });
    } catch (e) {
      set({ view: { kind: "welcome" } });
      get().toastError(e);
    }
    // playit не обязателен для работы: без интернета просто покажем сохранённые адреса.
    api.playitStatus().then(
      (playit) => set({ playit }),
      (e) => console.warn("Не удалось получить состояние playit", e),
    );
  },

  async refreshServers() {
    const list = await api.listServers();
    set({ servers: Object.fromEntries(list.map((s) => [s.meta.id, s])) });
  },

  async loadConsole(id) {
    const snapshot = await api.console(id);
    set((st) => {
      // Живые строки, пришедшие, пока мы ждали снимок, тоже сохраняем.
      const last = snapshot.length ? snapshot[snapshot.length - 1].seq : -1;
      const live = (st.consoles[id] ?? []).filter((l) => l.seq > last);
      return { consoles: { ...st.consoles, [id]: [...snapshot, ...live].slice(-CONSOLE_LIMIT) } };
    });
  },

  navigate(view) {
    set({ view });
  },

  openServer(id, tab = "overview") {
    set({ view: { kind: "server", id, tab } });
  },

  setConfig(config) {
    set({ config });
  },

  setAppSettingsOpen(open) {
    set({ appSettingsOpen: open });
  },

  toast(kind, text) {
    const id = ++toastSeq;
    set((st) => ({ toasts: [...st.toasts, { id, kind, text }] }));
    setTimeout(() => get().dismissToast(id), kind === "error" ? 9000 : 3500);
  },

  toastError(e) {
    get().toast("error", errorMessage(e));
  },

  dismissToast(id) {
    set((st) => ({ toasts: st.toasts.filter((t) => t.id !== id) }));
  },

  removeServer(id) {
    set((st) => {
      const servers = { ...st.servers };
      delete servers[id];
      const remaining = Object.keys(servers);
      return {
        servers,
        view: remaining.length ? { kind: "server", id: remaining[0], tab: "overview" } : { kind: "welcome" },
      };
    });
  },
}));

function setStatus(serverId: string, status: ServerStatus) {
  useHearth.setState((st) => {
    const existing = st.servers[serverId];
    const runningSince = { ...st.runningSince };
    if (status.state === "running") runningSince[serverId] ??= Date.now();
    else delete runningSince[serverId];
    if (!existing) {
      // Новый сервер (только что создан): подтянем его метаданные.
      void useHearth.getState().refreshServers();
      return { runningSince };
    }
    return { servers: { ...st.servers, [serverId]: { ...existing, status } }, runningSince };
  });
  // При запуске ядро очищает консоль: берём свежий снимок вместо старых строк.
  if (status.state === "starting") void useHearth.getState().loadConsole(serverId);
  // После установки в метаданных появились jar, версия Java и т.п.
  if (status.state === "stopped" || status.state === "starting") {
    void api.getServer(serverId).then(
      (s) => useHearth.setState((st) => ({ servers: { ...st.servers, [serverId]: s } })),
      () => {},
    );
  }
}

function handleEvent(event: Event) {
  switch (event.type) {
    case "console": {
      const { serverId, line } = event.data;
      useHearth.setState((st) => {
        const lines = st.consoles[serverId] ?? [];
        const next = lines.length >= CONSOLE_LIMIT ? [...lines.slice(1 - CONSOLE_LIMIT), line] : [...lines, line];
        return { consoles: { ...st.consoles, [serverId]: next } };
      });
      break;
    }
    case "status":
      setStatus(event.data.serverId, event.data.status);
      break;
    case "install":
      useHearth.setState((st) => ({ install: { ...st.install, [event.data.serverId]: event.data.progress } }));
      break;
    case "playit":
      onPlayit(event.data);
      break;
    case "stats": {
      const { serverId, stats } = event.data;
      useHearth.setState((st) => {
        const existing = st.servers[serverId];
        return existing ? { servers: { ...st.servers, [serverId]: { ...existing, stats } } } : {};
      });
      break;
    }
    case "backups": {
      const { serverId, inProgress } = event.data;
      useHearth.setState((st) => ({
        backupBusy: { ...st.backupBusy, [serverId]: inProgress },
        backupsRev: { ...st.backupsRev, [serverId]: (st.backupsRev[serverId] ?? 0) + 1 },
      }));
      break;
    }
    case "direct": {
      const { serverId, info } = event.data;
      useHearth.setState((st) => {
        const existing = st.servers[serverId];
        return existing ? { servers: { ...st.servers, [serverId]: { ...existing, direct: info } } } : {};
      });
      // Ядро могло само включить доступ (прямой или через playit): перечитываем сервер.
      if (info.state !== "opening") {
        void api.getServer(serverId).then(
          (s) => useHearth.setState((st) => ({ servers: { ...st.servers, [serverId]: s } })),
          () => {},
        );
      }
      break;
    }
  }
}

function setMeta(meta: ServerSummary["meta"]) {
  useHearth.setState((st) => {
    const existing = st.servers[meta.id];
    return existing ? { servers: { ...st.servers, [meta.id]: { ...existing, meta } } } : {};
  });
}

function onPlayit(playit: PlayitStatus) {
  const { pendingAccess, enableFriendAccess, servers, refreshServers } = useHearth.getState();
  useHearth.setState({ playit });
  // Ядро само создаёт туннели запущенным серверам: если появился неизвестный туннель, перечитываем серверы.
  const known = new Set(Object.values(servers).map((s) => s.meta.playit?.tunnelId));
  if (playit.tunnels.some((tun) => !known.has(tun.id))) void refreshServers();
  if (!pendingAccess) return;
  if (playit.linked) {
    // Пользователь подтвердил подключение в браузере: сразу создаём адрес.
    useHearth.setState({ pendingAccess: null });
    void enableFriendAccess(pendingAccess);
  } else if (!playit.claimUrl) {
    // Подключение отменили или оно не удалось (ошибка видна на карточке).
    useHearth.setState({ pendingAccess: null });
  }
}

/** Сервер выбран в боковой панели. */
export function useCurrentServer(): ServerSummary | null {
  return useHearth((st) => (st.view.kind === "server" ? (st.servers[st.view.id] ?? null) : null));
}
