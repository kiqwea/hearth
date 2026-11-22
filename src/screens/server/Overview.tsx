import { useEffect, useState, type ReactNode } from "react";
import { Copy, Info, TriangleExclamationPoint } from "lucide-react";

import type { ServerSummary } from "../../bindings/ServerSummary";
import { ConsolePanel } from "../../components/ConsolePanel";
import { FriendAccess } from "./FriendAccess";
import { Card, IconButton } from "../../components/ui";
import { t } from "../../i18n";
import { copyText, formatGb, formatUptime, loaderName, withPort } from "../../lib/format";
import { useHearth } from "../../lib/store";

export function Overview({ server }: { server: ServerSummary }) {
  const { meta, status } = server;
  const network = useHearth((s) => s.network);
  const since = useHearth((s) => s.runningSince[meta.id]);
  const openServer = useHearth((s) => s.openServer);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, []);

  const tiles = [
    {
      label: t.overview.core,
      value: `${loaderName(meta.loader)} ${meta.gameVersion}`,
      sub: meta.loader === "paper" && meta.loaderVersion ? t.overview.build(meta.loaderVersion) : meta.loaderVersion,
    },
    { label: t.overview.java, value: meta.javaMajor !== null ? String(meta.javaMajor) : "—", sub: "Eclipse Temurin" },
    { label: t.overview.memory, value: formatGb(meta.memoryMb), sub: t.overview.memoryAllocated },
    {
      label: t.overview.uptime,
      value: status.state === "running" && since ? formatUptime(now - since) : t.overview.notRunning,
      sub: null,
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
            <span className="text-[22px] font-bold leading-tight">{tile.value}</span>
            {tile.sub && <span className="text-[13px] text-muted">{tile.sub}</span>}
          </div>
        ))}
      </section>

      <Card label={t.overview.connect} className="px-5 py-[18px] flex flex-col gap-3.5">
        <h2 className="m-0 text-base font-semibold">{t.overview.connect}</h2>
        <FriendAccess server={server} />
        <div className="flex flex-wrap gap-2.5">
          <Address label={t.overview.lan} value={network?.lanIp ? withPort(network.lanIp, meta.port) : null} />
          <Address label={t.overview.local} value={withPort("localhost", meta.port)} />
        </div>
      </Card>

      <ConsolePanel id={meta.id} compact onOpenFull={() => openServer(meta.id, "console")} />
    </div>
  );
}

function Address({ label, value }: { label: string; value: string | null }) {
  const toast = useHearth((s) => s.toast);
  return (
    <div className="flex-[1_1_220px] flex items-center justify-between gap-2.5 px-3.5 py-2.5 border border-line rounded-[10px]">
      <span className="flex flex-col gap-0.5 min-w-0">
        <span className="text-xs text-muted">{label}</span>
        <span className={`font-mono selectable truncate ${value ? "" : "text-faint"}`}>{value ?? t.overview.lanUnknown}</span>
      </span>
      {value && (
        <IconButton
          label={`${t.actions.copy}: ${label}`}
          onClick={async () => {
            if (await copyText(value)) toast("success", `${t.actions.copied}: ${value}`);
          }}
        >
          <Copy size={16} aria-hidden="true" />
        </IconButton>
      )}
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
