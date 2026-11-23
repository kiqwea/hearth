import { useEffect, useState } from "react";
import { FolderOpen, Play, RotateCw, Square } from "lucide-react";

import type { ServerSummary } from "../../bindings/ServerSummary";
import { Button, Chip, Dialog, IconButton, Spinner, StatusPill } from "../../components/ui";
import { t } from "../../i18n";
import { api, isHearthError } from "../../lib/api";
import { formatGb, formatUptime, isModded, isPaperLike, serverTitle, statusText, statusTone } from "../../lib/format";
import { useHearth, type ServerTab } from "../../lib/store";
import { BackupsTab } from "./BackupsTab";
import { ConsoleTab } from "./ConsoleTab";
import { ModsTab } from "./ModsTab";
import { Overview } from "./Overview";
import { PlayersTab } from "./PlayersTab";
import { SettingsTab } from "./SettingsTab";

export function ServerView({ id, tab }: { id: string; tab: ServerTab }) {
  const server = useHearth((s) => s.servers[id]);
  const openServer = useHearth((s) => s.openServer);

  if (!server) {
    return (
      <div className="h-full flex items-center justify-center">
        <Spinner />
      </div>
    );
  }

  const extensions = isPaperLike(server.meta.loader) ? t.tabs.plugins : isModded(server.meta.loader) ? t.tabs.mods : null;
  const tabs: Array<{ key: ServerTab | null; label: string }> = [
    { key: "overview", label: t.tabs.overview },
    { key: "console", label: t.tabs.console },
    { key: "players", label: t.tabs.players },
    ...(extensions ? [{ key: "mods" as const, label: extensions }] : []),
    { key: "backups", label: t.tabs.backups },
    { key: "settings", label: t.tabs.settings },
  ];

  return (
    <div className={`px-8 py-7 flex flex-col gap-[22px] ${tab === "console" ? "h-full" : ""}`}>
      <Header server={server} />

      <nav aria-label="Разделы сервера" className="flex flex-wrap gap-1 border-b border-line">
        {tabs.map(({ key, label }) =>
          key ? (
            <button
              key={label}
              type="button"
              aria-current={tab === key ? "page" : undefined}
              onClick={() => openServer(id, key)}
              className={`px-3.5 py-2.5 -mb-px border-b-2 transition ${
                tab === key ? "text-ink font-semibold border-accent" : "text-muted border-transparent hover:text-ink"
              }`}
            >
              {label}
            </button>
          ) : (
            <span key={label} title={t.app.soon} className="px-3.5 py-2.5 text-faint/70 cursor-default">
              {label}
              <span className="ml-1.5 text-[11px] px-1.5 py-px rounded-full bg-chip text-faint">{t.app.soon}</span>
            </span>
          ),
        )}
      </nav>

      {tab === "overview" && <Overview server={server} />}
      {tab === "console" && <ConsoleTab id={id} />}
      {tab === "players" && <PlayersTab server={server} />}
      {tab === "mods" && extensions && <ModsTab server={server} />}
      {tab === "backups" && <BackupsTab server={server} />}
      {tab === "settings" && <SettingsTab server={server} />}
    </div>
  );
}

/** Перерисовка раз в 30 секунд для счётчика времени работы. */
function useNow(intervalMs = 30_000) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs]);
  return now;
}

function Header({ server }: { server: ServerSummary }) {
  const { meta, status } = server;
  const since = useHearth((s) => s.runningSince[meta.id]);
  const toastError = useHearth((s) => s.toastError);
  const now = useNow();
  const [pending, setPending] = useState(false);
  const [killDialog, setKillDialog] = useState<null | "manual" | "timeout">(null);

  const run = async (action: () => Promise<void>) => {
    setPending(true);
    try {
      await action();
    } catch (e) {
      if (isHearthError(e) && e.kind === "stopTimeout") setKillDialog("timeout");
      else toastError(e);
    } finally {
      setPending(false);
    }
  };

  const state = status.state;
  let pillText = statusText(status);
  if (state === "running" && since) pillText = `${pillText} · ${formatUptime(now - since)}`;

  return (
    <header className="flex flex-wrap items-start justify-between gap-4">
      <div className="flex flex-col gap-2.5 min-w-0">
        <h1 className="m-0 text-[28px] font-bold tracking-tight break-words">{meta.name}</h1>
        <div className="flex flex-wrap items-center gap-2">
          <StatusPill tone={statusTone(status)}>{pillText}</StatusPill>
          <Chip>{serverTitle(meta)}</Chip>
          {meta.javaMajor !== null && <Chip>Java {meta.javaMajor}</Chip>}
          <Chip>{formatGb(meta.memoryMb)} памяти</Chip>
        </div>
      </div>

      <div className="flex flex-wrap gap-2">
        {(state === "stopped" || state === "crashed") && (
          <Button variant="primary" disabled={pending} onClick={() => run(() => api.start(meta.id))}>
            <Play size={16} fill="currentColor" aria-hidden="true" />
            {t.actions.start}
          </Button>
        )}
        {state === "installing" && (
          <Button variant="secondary" disabled>
            <Spinner className="!w-4 !h-4 !border-2" />
            {t.status.installing}
          </Button>
        )}
        {(state === "running" || state === "starting") && (
          <>
            <Button variant="secondary" disabled={pending} onClick={() => run(() => api.stop(meta.id))}>
              <Square size={14} fill="currentColor" aria-hidden="true" />
              {t.actions.stop}
            </Button>
            <Button variant="ghost" disabled={pending || state === "starting"} onClick={() => run(() => api.restart(meta.id))}>
              <RotateCw size={16} aria-hidden="true" />
              {t.actions.restart}
            </Button>
          </>
        )}
        {state === "stopping" && (
          <>
            <Button variant="secondary" disabled>
              <Spinner className="!w-4 !h-4 !border-2" />
              {t.status.stopping}
            </Button>
            <Button variant="danger" onClick={() => setKillDialog("manual")}>
              {t.actions.kill}
            </Button>
          </>
        )}
        <IconButton label={t.actions.openFolder} onClick={() => run(() => api.openFolder(meta.id))} className="!w-11 !h-11 border border-line-strong !bg-transparent">
          <FolderOpen size={18} aria-hidden="true" />
        </IconButton>
      </div>

      <Dialog
        open={killDialog !== null}
        onClose={() => setKillDialog(null)}
        title={killDialog === "timeout" ? t.kill.timeoutTitle : t.kill.title}
        footer={
          <>
            <Button
              variant="ghost"
              onClick={() => {
                setKillDialog(null);
                if (killDialog === "timeout") void run(() => api.stop(meta.id));
              }}
            >
              {t.kill.wait}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                setKillDialog(null);
                void run(() => api.kill(meta.id));
              }}
            >
              {t.actions.kill}
            </Button>
          </>
        }
      >
        <p className="m-0">{t.kill.text}</p>
      </Dialog>
    </header>
  );
}
