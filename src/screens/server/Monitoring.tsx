// Верх вкладки «Мониторинг»: графики памяти, процессора, TPS и игроков.
// Память — главное: по ней видно, хватает ли выделенного объёма.

import { useEffect, useMemo, useState } from "react";
import { CheckCircle2, Info, TriangleAlert } from "lucide-react";

import type { ServerSummary } from "../../bindings/ServerSummary";
import { Card, Segmented } from "../../components/ui";
import { TimeChart, type ChartPoint } from "../../components/TimeChart";
import { t } from "../../i18n";
import { formatGb } from "../../lib/format";
import { useHearth } from "../../lib/store";

const WINDOWS = [5, 15, 60] as const;
type Window = (typeof WINDOWS)[number];

/** Перерисовка раз в 5 секунд, чтобы ось времени ехала. */
function useNow() {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 5000);
    return () => clearInterval(timer);
  }, []);
  return now;
}

/** «2,1 ГБ» для МБ. */
const gbText = (mb: number) => formatGb(Math.round(mb));

/** Округление вверх до целых ГБ (как в слайдере памяти), не меньше 2 ГБ. */
const roundGb = (mb: number) => Math.max(2, Math.ceil(mb / 1024)) * 1024;

export function Monitoring({ server }: { server: ServerSummary }) {
  const { meta, status } = server;
  const history = useHearth((s) => s.history[meta.id]);
  const loadHistory = useHearth((s) => s.loadHistory);
  const [win, setWin] = useState<Window>(5);
  const now = useNow();
  const running = status.state === "running" || status.state === "starting";

  useEffect(() => {
    void loadHistory(meta.id);
  }, [meta.id, loadHistory, status.state]);

  const from = now - win * 60_000;
  const samples = useMemo(() => (history ?? []).filter((s) => s.at >= from - 20_000), [history, from]);
  const allocated = meta.memoryMb;

  const memory: ChartPoint[] = samples.map((s) => ({ at: s.at, value: s.heapUsedMb }));
  const peaks = new Map(samples.map((s) => [s.at, s.heapPeakMb]));
  const used = samples.map((s) => s.heapUsedMb).filter((v): v is number => v !== null);
  const current = used.length ? used[used.length - 1] : null;
  const latest = samples.length ? samples[samples.length - 1] : null;
  const maxUsed = used.length ? Math.max(...used) : null;

  // Вывод о памяти: по наибольшей «живой» занятости за период.
  let verdict: { tone: "ok" | "warn" | "info" | "wait"; text: string };
  if (maxUsed === null || used.length < 12) verdict = { tone: "wait", text: t.monitor.verdictWait };
  else if (maxUsed > allocated * 0.8) verdict = { tone: "warn", text: t.monitor.verdictMore(gbText(roundGb(maxUsed * 1.5))) };
  else if (maxUsed < allocated * 0.35 && allocated > 2048 && roundGb(maxUsed * 2) < allocated)
    verdict = { tone: "info", text: t.monitor.verdictLess(gbText(roundGb(maxUsed * 2))) };
  else verdict = { tone: "ok", text: t.monitor.verdictOk };

  const gbTicks = (max: number) => {
    const stepGb = max > 12 * 1024 ? 4 : max > 6 * 1024 ? 2 : 1;
    const out: number[] = [];
    for (let v = 0; v <= max; v += stepGb * 1024) out.push(v);
    return out;
  };
  const timeLabels: [string, string, string] = [t.monitor.ago(win), t.monitor.ago(win / 2), t.monitor.now];
  const maxPlayers = Math.max(server.stats?.maxPlayers ?? 0, ...samples.map((s) => s.online), 1);

  if (!running && samples.length === 0) {
    return <p className="m-0 px-4 py-3 rounded-xl bg-chip text-soft">{t.monitor.stopped}</p>;
  }

  const VerdictIcon = verdict.tone === "warn" ? TriangleAlert : verdict.tone === "ok" ? CheckCircle2 : Info;
  const verdictTone = verdict.tone === "warn" ? "text-warn" : verdict.tone === "ok" ? "text-ok-text" : "text-muted";

  return (
    <div className="flex flex-col gap-[18px]">
      <Card label={t.monitor.memory} className="px-5 py-4 flex flex-col gap-3">
        <div className="flex flex-wrap items-start gap-4">
          <span className="flex-[1_1_260px] flex flex-col gap-1">
            <h2 className="m-0 text-base font-semibold">{t.monitor.memory}</h2>
            <span className="text-[28px] font-bold tracking-tight leading-none">
              {current !== null ? t.monitor.memoryNow(gbText(current), gbText(allocated)) : "—"}
            </span>
            <span className={`flex items-center gap-1.5 text-[13px] ${verdictTone}`}>
              <VerdictIcon size={15} aria-hidden="true" />
              {verdict.text}
            </span>
          </span>
          <Segmented
            label={t.monitor.window}
            value={String(win)}
            onChange={(v) => setWin(Number(v) as Window)}
            options={WINDOWS.map((m) => ({ value: String(m), label: t.monitor.minutes(m) }))}
          />
        </div>
        {used.length === 0 ? (
          <p className="m-0 py-10 text-center text-[13px] text-muted">{t.monitor.waitingGc}</p>
        ) : (
          <TimeChart
            points={memory}
            from={from}
            to={now}
            yMax={allocated * 1.08}
            ticks={gbTicks(allocated)}
            format={gbText}
            label={`${t.monitor.memory}: ${current !== null ? gbText(current) : "—"} / ${gbText(allocated)}`}
            height={210}
            reference={{ value: allocated, label: t.monitor.allocated(gbText(allocated)) }}
            tooltip={(p) => {
              const peak = peaks.get(p.at);
              return peak ? [t.monitor.peak(gbText(peak))] : [];
            }}
            timeLabels={timeLabels}
          />
        )}
        <span className="text-[12px] text-faint">{t.monitor.memoryHint}</span>
      </Card>

      <div className="grid gap-[18px] grid-cols-[repeat(auto-fit,minmax(260px,1fr))]">
        <SmallChart title={t.monitor.cpu} value={latest?.cpuPercent ?? null} format={(v) => `${Math.round(v)}%`}>
          <TimeChart
            points={samples.map((s) => ({ at: s.at, value: s.cpuPercent }))}
            from={from}
            to={now}
            yMax={100}
            ticks={[0, 50, 100]}
            format={(v) => `${Math.round(v)}%`}
            label={t.monitor.cpu}
            height={120}
            timeLabels={timeLabels}
          />
        </SmallChart>
        <SmallChart title={t.monitor.tps} value={latest?.tps ?? null} format={(v) => v.toFixed(1).replace(".", ",")}>
          <TimeChart
            points={samples.map((s) => ({ at: s.at, value: s.tps }))}
            from={from}
            to={now}
            yMax={21}
            ticks={[0, 10, 20]}
            format={(v) => (Number.isInteger(v) ? String(v) : v.toFixed(1).replace(".", ","))}
            label={t.monitor.tps}
            height={120}
            reference={{ value: 20, label: t.monitor.tpsNorm }}
            timeLabels={timeLabels}
          />
        </SmallChart>
        <SmallChart title={t.monitor.players} value={latest?.online ?? null} format={(v) => String(v)}>
          <TimeChart
            points={samples.map((s) => ({ at: s.at, value: s.online }))}
            from={from}
            to={now}
            yMax={maxPlayers * 1.15}
            ticks={[0, maxPlayers]}
            format={(v) => String(Math.round(v))}
            label={t.monitor.players}
            height={120}
            step
            timeLabels={timeLabels}
          />
        </SmallChart>
      </div>
    </div>
  );
}

function SmallChart({
  title,
  value,
  format,
  children,
}: {
  title: string;
  value: number | null;
  format: (v: number) => string;
  children: React.ReactNode;
}) {
  return (
    <Card label={title} className="px-4 py-3.5 flex flex-col gap-2 min-w-0">
      <div className="flex items-baseline justify-between gap-2">
        <h3 className="m-0 text-[13px] font-semibold text-soft">{title}</h3>
        <span className="text-lg font-bold">{value !== null ? format(value) : "—"}</span>
      </div>
      {children}
    </Card>
  );
}
