import { useEffect, useState, type ReactNode } from "react";
import { Info, TriangleExclamationPoint } from "lucide-react";

import type { ServerSummary } from "../../bindings/ServerSummary";
import { ConsolePanel } from "../../components/ConsolePanel";
import { FriendAccess } from "./FriendAccess";
import { Card } from "../../components/ui";
import { t } from "../../i18n";
import { formatGb, formatUptime, loaderName } from "../../lib/format";
import { useHearth } from "../../lib/store";

export function Overview({ server }: { server: ServerSummary }) {
  const { meta, status } = server;
  const since = useHearth((s) => s.runningSince[meta.id]);
  const openServer = useHearth((s) => s.openServer);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, []);

  // Статистика приходит раз в несколько секунд, пока сервер работает.
  const stats = status.state === "running" ? server.stats : null;
  const tps = stats?.tps ?? null;
  const tpsTone = tps === null ? "" : tps >= 19 ? "text-ok" : tps >= 15 ? "text-warn" : "text-err";
  const tiles: Array<{ label: string; value: string; sub: string | null; tone?: string }> = [
    {
      label: t.overview.players,
      value: stats?.maxPlayers != null ? `${stats.online.length} / ${stats.maxPlayers}` : "—",
      sub: stats ? (stats.online.length ? stats.online.join(", ") : t.overview.nobody) : null,
    },
    {
      label: t.overview.tps,
      value: tps !== null ? tps.toFixed(1) : "—",
      sub: tps === null ? (stats ? t.overview.tpsUnknown : null) : tps >= 19 ? t.overview.tpsOk : tps >= 15 ? t.overview.tpsLag : t.overview.tpsBad,
      tone: tpsTone,
    },
    {
      label: t.overview.cpu,
      value: stats?.cpuPercent != null ? `${Math.round(stats.cpuPercent)}%` : "—",
      sub: stats?.cpuPercent != null ? t.overview.cpuSub : null,
    },
    {
      label: t.overview.memory,
      // Реальная занятость миром (из GC-лога): процесс Java всегда держит всю выделенную память.
      value: stats?.heapUsedMb != null ? formatGb(stats.heapUsedMb) : formatGb(meta.memoryMb),
      sub: stats?.heapUsedMb != null ? t.overview.memoryUsed(formatGb(meta.memoryMb)) : t.overview.memoryAllocated,
    },
    {
      label: t.overview.uptime,
      value: status.state === "running" && since ? formatUptime(now - since) : t.overview.notRunning,
      sub: `${loaderName(meta.loader)} ${meta.gameVersion}`,
    },
  ];

  return (
    <div className="flex flex-col gap-[22px]">
      {status.state === "stopped" && (
        <Banner tone="info">{t.overview.stoppedHint}</Banner>
      )}
      {status.state === "crashed" && (
        <Banner tone="error">
          {t.overview.crashedHint}{" "}
          <button type="button" onClick={() => openServer(meta.id, "console")} className="underline font-semibold">
            {t.overview.openConsole}
          </button>
        </Banner>
      )}

      <section aria-label="Сводка" className="grid gap-3.5 grid-cols-[repeat(auto-fit,minmax(180px,1fr))]">
        {tiles.map((tile) => (
          <div key={tile.label} className="bg-card border border-line rounded-[14px] px-[18px] py-4 flex flex-col gap-1.5">
            <span className="text-muted text-[13px]">{tile.label}</span>
            <span className={`text-[22px] font-bold leading-tight ${tile.tone ?? ""}`}>{tile.value}</span>
            {tile.sub && <span className="text-[13px] text-muted truncate" title={tile.sub}>{tile.sub}</span>}
          </div>
        ))}
      </section>

      <Card label={t.overview.connect} className="px-5 py-[18px] flex flex-col gap-3.5">
        <h2 className="m-0 text-base font-semibold">{t.overview.connect}</h2>
        <FriendAccess server={server} />
      </Card>

      <ConsolePanel id={meta.id} compact onOpenFull={() => openServer(meta.id, "console")} />
    </div>
  );
}

function Banner({ tone, children }: { tone: "info" | "error"; children: ReactNode }) {
  const Icon = tone === "error" ? TriangleExclamationPoint : Info;
  return (
    <div
      role={tone === "error" ? "alert" : undefined}
      className={`flex items-center gap-2.5 px-4 py-3 rounded-xl ${tone === "error" ? "bg-err-bg text-err" : "bg-accent-soft text-accent-text"}`}
    >
      <Icon size={18} className="shrink-0" aria-hidden="true" />
      <span className="flex-1">{children}</span>
    </div>
  );
}
