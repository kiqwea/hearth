import { useState } from "react";
import { TriangleExclamationPoint } from "lucide-react";

import { t } from "../i18n";
import { api, errorMessage } from "../lib/api";
import { isActive } from "../lib/format";
import { useHearth } from "../lib/store";
import { Button, Dialog, ProgressBar } from "./ui";

type Phase = "idle" | "stopping" | "downloading" | "installing";

export function UpdateDialog() {
  const update = useHearth((s) => s.update);
  const open = useHearth((s) => s.updateDialogOpen);
  const setOpen = useHearth((s) => s.setUpdateDialogOpen);
  const servers = useHearth((s) => s.servers);
  const toast = useHearth((s) => s.toast);

  const [phase, setPhase] = useState<Phase>("idle");
  const [progress, setProgress] = useState<{ done: number; total: number | null } | null>(null);

  if (!update) return null;
  const running = Object.values(servers).filter((s) => isActive(s.status)).length;
  const busy = phase !== "idle";

  async function install() {
    if (!update) return;
    try {
      if (running > 0) {
        setPhase("stopping");
        await api.stopAll();
      }
      setPhase("downloading");
      await update.install((done, total) => {
        setProgress({ done, total });
        if (total !== null && done >= total) setPhase("installing");
      });
      setPhase("installing");
    } catch (e) {
      setPhase("idle");
      setProgress(null);
      toast("error", `${t.update.checkFailed}: ${errorMessage(e)}`);
    }
  }

  const status =
    phase === "stopping" ? t.update.stopping : phase === "downloading" ? t.update.downloading : phase === "installing" ? t.update.installing : null;

  return (
    <Dialog
      open={open}
      onClose={() => !busy && setOpen(false)}
      title={t.update.title(update.version)}
      wide
      footer={
        <>
          <Button variant="ghost" onClick={() => setOpen(false)} disabled={busy}>
            {t.update.later}
          </Button>
          <Button variant="primary" onClick={install} disabled={busy}>
            {t.update.install}
          </Button>
        </>
      }
    >
      <p className="m-0">
        {t.update.current(update.currentVersion)} {t.update.keepData}
      </p>
      {update.notes && (
        <div className="flex flex-col gap-1.5">
          <span className="font-semibold text-ink">{t.update.whatsNew}</span>
          <div className="selectable whitespace-pre-wrap text-[13px] leading-relaxed bg-bg border border-line rounded-[10px] px-3.5 py-3 max-h-[220px] overflow-y-auto">
            {update.notes}
          </div>
        </div>
      )}
      {running > 0 && !busy && (
        <div className="flex gap-2.5 items-start px-3.5 py-3 rounded-[10px] bg-warn-bg text-warn">
          <TriangleExclamationPoint size={18} className="shrink-0 mt-0.5" aria-hidden="true" />
          <span>{t.update.serversWarning(running)}</span>
        </div>
      )}
      {status && (
        <div className="flex flex-col gap-2">
          <span className="text-ink font-semibold">{status}</span>
          {phase !== "stopping" && (
            <ProgressBar
              value={progress?.total ? (progress.done / progress.total) * 100 : null}
              label={t.update.downloading}
            />
          )}
        </div>
      )}
    </Dialog>
  );
}
