// «Ускорить сервер» (пресет «Производительность») и предгенерация мира через Chunky.
// Живут на вкладке «Плагины» / «Моды»: оба ставят моды или плагины.

import { useEffect, useRef, useState } from "react";
import { Gauge, Map } from "lucide-react";

import type { PregenStatus } from "../../bindings/PregenStatus";
import type { PresetResult } from "../../bindings/PresetResult";
import type { ServerSummary } from "../../bindings/ServerSummary";
import { Button, Card, Dialog, ProgressBar, Spinner } from "../../components/ui";
import { t } from "../../i18n";
import { api, isHearthError } from "../../lib/api";
import { isPaperLike } from "../../lib/format";
import { useHearth } from "../../lib/store";

const RADII = [500, 1000, 2000, 3000, 5000];

export function PresetCard({ server, onApplied }: { server: ServerSummary; onApplied: () => void }) {
  const { meta } = server;
  const toastError = useHearth((s) => s.toastError);
  const [working, setWorking] = useState(false);
  const [result, setResult] = useState<PresetResult | null>(null);

  async function apply() {
    setWorking(true);
    try {
      setResult(await api.applyPreset(meta.id));
      onApplied();
    } catch (e) {
      toastError(e);
    } finally {
      setWorking(false);
    }
  }

  const changed = result ? result.installed.length + result.settings.length : 0;

  return (
    <Card label={t.preset.title} className="px-5 py-4 flex flex-wrap items-center gap-4">
      <span aria-hidden="true" className="w-10 h-10 rounded-lg bg-accent-soft text-accent flex items-center justify-center shrink-0">
        <Gauge size={20} />
      </span>
      <span className="flex-[1_1_300px] flex flex-col gap-0.5">
        <h2 className="m-0 text-base font-semibold">{t.preset.title}</h2>
        <span className="text-[13px] text-muted">{isPaperLike(meta.loader) ? t.preset.hintPaper : t.preset.hintFabric}</span>
      </span>
      <Button variant="primary" disabled={working} onClick={() => void apply()}>
        {working ? (
          <>
            <Spinner className="!w-4 !h-4 !border-2" />
            {t.preset.applying}
          </>
        ) : (
          t.preset.apply
        )}
      </Button>

      <Dialog
        open={result !== null}
        onClose={() => setResult(null)}
        title={t.preset.doneTitle}
        footer={
          <Button variant="primary" onClick={() => setResult(null)}>
            {t.actions.close}
          </Button>
        }
      >
        {result && (
          <>
            {changed === 0 && !result.paperConfigMissing && result.skipped.length === 0 && <p className="m-0">{t.preset.nothing}</p>}
            <ResultList title={t.preset.installed} items={result.installed} />
            <ResultList title={t.preset.settings} items={result.settings} />
            <ResultList title={t.preset.already} items={result.already} />
            <ResultList title={t.preset.skipped} items={result.skipped} muted />
            {result.paperConfigMissing && <p className="m-0 text-warn">{t.preset.paperMissing}</p>}
            {changed > 0 && <p className="m-0 text-accent-text">{t.preset.restart}</p>}
          </>
        )}
      </Dialog>
    </Card>
  );
}

function ResultList({ title, items, muted }: { title: string; items: string[]; muted?: boolean }) {
  if (items.length === 0) return null;
  return (
    <div>
      <p className="m-0 font-semibold text-ink">{title}</p>
      <ul className={`m-0 pl-5 list-disc text-[13px] ${muted ? "text-muted" : ""}`}>
        {items.map((i) => (
          <li key={i}>{i}</li>
        ))}
      </ul>
    </div>
  );
}

export function PregenCard({
  server,
  chunkyInstalled,
  installing,
  onInstallChunky,
}: {
  server: ServerSummary;
  chunkyInstalled: boolean;
  installing: boolean;
  onInstallChunky: () => void;
}) {
  const { meta, status } = server;
  const toast = useHearth((s) => s.toast);
  const toastError = useHearth((s) => s.toastError);
  const [radius, setRadius] = useState(2000);
  const [progress, setProgress] = useState<PregenStatus | null>(null);
  const [notLoaded, setNotLoaded] = useState(false);
  const [busy, setBusy] = useState(false);
  const wasRunning = useRef(false);
  const running = status.state === "running";

  // Пока идёт генерация, спрашиваем прогресс раз в 2 секунды.
  useEffect(() => {
    if (!running || !chunkyInstalled) {
      setProgress(null);
      wasRunning.current = false;
      return;
    }
    let stopped = false;
    const poll = async () => {
      try {
        const s = await api.pregenStatus(meta.id);
        if (stopped) return;
        setNotLoaded(false);
        if (wasRunning.current && !s.running) toast("success", t.pregen.finished);
        wasRunning.current = s.running;
        setProgress(s);
      } catch (e) {
        if (stopped) return;
        // Chunky поставлен на работающий сервер, но ещё не загружен.
        if (isHearthError(e) && e.message.includes("перезапустите")) setNotLoaded(true);
      }
    };
    void poll();
    const timer = setInterval(() => void poll(), 2000);
    return () => {
      stopped = true;
      clearInterval(timer);
    };
  }, [running, chunkyInstalled, meta.id, toast]);

  async function start() {
    setBusy(true);
    try {
      const s = await api.pregenStart(meta.id, radius);
      wasRunning.current = s.running;
      setProgress(s);
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  }

  async function cancel() {
    setBusy(true);
    try {
      await api.pregenCancel(meta.id);
      wasRunning.current = false;
      setProgress(null);
      toast("info", t.pregen.cancelled);
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(false);
    }
  }

  const active = progress?.running ?? false;
  const pct = progress?.percent ?? 0;

  return (
    <Card label={t.pregen.title} className="px-5 py-4 flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-4">
        <span aria-hidden="true" className="w-10 h-10 rounded-lg bg-chip text-soft flex items-center justify-center shrink-0">
          <Map size={20} />
        </span>
        <span className="flex-[1_1_300px] flex flex-col gap-0.5">
          <h2 className="m-0 text-base font-semibold">{t.pregen.title}</h2>
          <span className="text-[13px] text-muted">{t.pregen.hint}</span>
        </span>
      </div>

      {!chunkyInstalled ? (
        <div className="flex flex-wrap items-center gap-3">
          <span className="flex-1 text-[13px] text-soft">{t.pregen.needChunky}</span>
          <Button size="sm" className="!h-10" disabled={installing} onClick={onInstallChunky}>
            {installing ? <Spinner className="!w-4 !h-4 !border-2" /> : null}
            {t.pregen.installChunky}
          </Button>
        </div>
      ) : notLoaded ? (
        <p className="m-0 text-[13px] text-accent-text">{t.pregen.chunkyRestart}</p>
      ) : active ? (
        <div className="flex flex-col gap-2">
          <ProgressBar value={pct} label={t.pregen.title} />
          <div className="flex flex-wrap items-center gap-3">
            <span className="flex-1 text-[13px] text-soft">
              {t.pregen.running(pct.toFixed(1).replace(".", ","), progress?.eta && progress.eta !== "0:00:00" ? progress.eta : null)}
            </span>
            <Button size="sm" variant="ghost" className="!h-10" disabled={busy} onClick={() => void cancel()}>
              {t.pregen.cancel}
            </Button>
          </div>
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-3">
          <label htmlFor="pregen-radius" className="text-[13px] text-soft">
            {t.pregen.radius}
          </label>
          <select
            id="pregen-radius"
            value={radius}
            onChange={(e) => setRadius(Number(e.target.value))}
            className="h-10 px-3 rounded-[9px] border border-line-strong bg-bg text-ink w-[180px]"
          >
            {RADII.map((r) => (
              <option key={r} value={r}>
                {t.pregen.blocks(r)}
              </option>
            ))}
          </select>
          <span className="flex-1 text-[13px] text-muted">{running ? "" : t.pregen.needRunning}</span>
          <Button size="sm" variant="secondary" className="!h-10" disabled={!running || busy} onClick={() => void start()}>
            {busy ? <Spinner className="!w-4 !h-4 !border-2" /> : null}
            {t.pregen.start}
          </Button>
        </div>
      )}
    </Card>
  );
}
