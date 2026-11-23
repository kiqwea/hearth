// «Версия сервера» в настройках: новая сборка ядра той же версии игры или
// переход на новую версию Minecraft с бэкапом и обновлением модов.

import { useCallback, useEffect, useState } from "react";
import { ArrowUpCircle, CheckCircle2 } from "lucide-react";

import type { ModCompat } from "../../bindings/ModCompat";
import type { ServerSummary } from "../../bindings/ServerSummary";
import type { UpdateInfo } from "../../bindings/UpdateInfo";
import type { UpdatePlan } from "../../bindings/UpdatePlan";
import type { UpdateResult } from "../../bindings/UpdateResult";
import { Button, Card, Dialog, Spinner } from "../../components/ui";
import { t } from "../../i18n";
import { api, errorMessage } from "../../lib/api";
import { isActive, isModded, isPaperLike, loaderName } from "../../lib/format";
import { useHearth } from "../../lib/store";

export function ServerVersion({ server }: { server: ServerSummary }) {
  const { meta, status } = server;
  const refreshServers = useHearth((s) => s.refreshServers);
  const toast = useHearth((s) => s.toast);
  const toastError = useHearth((s) => s.toastError);
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [target, setTarget] = useState<string | null>(null);
  const [plan, setPlan] = useState<UpdatePlan | null>(null);
  const [working, setWorking] = useState(false);
  const [result, setResult] = useState<UpdateResult | null>(null);

  const active = isActive(status);
  const installing = status.state === "installing";

  const check = useCallback(async () => {
    setFailed(null);
    try {
      setInfo(await api.checkServerUpdate(meta.id));
    } catch (e) {
      setFailed(errorMessage(e));
    }
  }, [meta.id]);

  // Проверяем при открытии и после каждого обновления (сменились версия или сборка).
  useEffect(() => {
    void check();
  }, [check, meta.gameVersion, meta.loaderVersion]);

  async function openPlan(version: string) {
    setTarget(version);
    setPlan(null);
    try {
      setPlan(await api.planUpdate(meta.id, version));
    } catch (e) {
      setTarget(null);
      toastError(e);
    }
  }

  async function run(version: string | null) {
    setTarget(null);
    setWorking(true);
    try {
      const r = await api.upgradeServer(meta.id, version);
      await refreshServers();
      if (r.notes.length || r.disabled.length) setResult(r);
      else toast("success", version ? t.upgrade.done(version) : t.upgrade.doneBuild);
    } catch (e) {
      toastError(e);
    } finally {
      setWorking(false);
    }
  }

  const title = `${loaderName(meta.loader)} ${meta.gameVersion}`;
  const fabric = meta.loader === "fabric";
  const newest = info?.newerVersions[0] ?? null;

  return (
    <Card label={t.upgrade.title} className="px-5 py-4 flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-3">
        <span className="flex-[1_1_280px] flex flex-col gap-0.5">
          <h2 className="m-0 text-base font-semibold">{t.upgrade.title}</h2>
          <span className="text-soft">{t.upgrade.current(title, meta.loaderVersion, fabric)}</span>
        </span>
      </div>

      {working || installing ? (
        <p className="m-0 flex items-center gap-2.5 text-accent-text" role="status">
          <Spinner className="!w-4 !h-4 !border-2" />
          {t.upgrade.updating}
        </p>
      ) : failed ? (
        <p className="m-0 flex flex-wrap items-center gap-3 text-[13px] text-muted">
          {t.upgrade.checkFailed}: {failed}
          <Button size="sm" variant="ghost" onClick={() => void check()}>
            {t.actions.retry}
          </Button>
        </p>
      ) : info === null ? (
        <p className="m-0 flex items-center gap-2.5 text-[13px] text-muted">
          <Spinner className="!w-4 !h-4 !border-2" />
          {t.upgrade.checking}
        </p>
      ) : !newest && !info.newBuild ? (
        <p className="m-0 flex items-center gap-2 text-[13px] text-ok-text">
          <CheckCircle2 size={16} aria-hidden="true" />
          {t.upgrade.upToDate}
        </p>
      ) : (
        <div className="flex flex-col gap-2.5">
          {newest && (
            <UpdateRow text={t.upgrade.newVersion(newest)} button={t.upgrade.updateTo(newest)} disabled={active} onClick={() => void openPlan(newest)} primary />
          )}
          {info.newBuild && (
            <UpdateRow
              text={t.upgrade.newBuild(info.newBuild, fabric)}
              button={t.upgrade.updateBuild(fabric)}
              disabled={active}
              onClick={() => void run(null)}
            />
          )}
          {active && <span className="text-[13px] text-muted">{t.upgrade.stopFirst}</span>}
        </div>
      )}

      <Dialog
        open={target !== null}
        onClose={() => setTarget(null)}
        wide
        title={target ? t.upgrade.planTitle(target) : ""}
        footer={
          <>
            <Button variant="ghost" onClick={() => setTarget(null)}>
              {t.actions.cancel}
            </Button>
            <Button variant="primary" disabled={!plan} onClick={() => target && void run(target)}>
              {t.upgrade.update}
            </Button>
          </>
        }
      >
        <p className="m-0">{t.upgrade.planText}</p>
        {plan === null ? (
          <p className="m-0 flex items-center gap-2.5 text-muted">
            <Spinner className="!w-4 !h-4 !border-2" />
            {t.upgrade.planChecking}
          </p>
        ) : plan.mods.length === 0 ? (
          isPaperLike(meta.loader) || isModded(meta.loader) ? <p className="m-0 text-muted">{t.upgrade.planNoMods}</p> : null
        ) : (
          <>
            <ul className="m-0 p-0 list-none flex flex-col gap-1.5 max-h-[300px] overflow-y-auto">
              {plan.mods.map((m) => (
                <PlanRow key={m.file} mod={m} disableMissing={plan.disableMissing} />
              ))}
            </ul>
            {plan.mods.some((m) => m.clientSide === "required") && <p className="m-0 text-[13px] text-warn">{t.upgrade.playersNote}</p>}
          </>
        )}
      </Dialog>

      <Dialog
        open={result !== null}
        onClose={() => setResult(null)}
        title={result ? t.upgrade.done(result.meta.gameVersion) : ""}
        footer={
          <Button variant="primary" onClick={() => setResult(null)}>
            {t.actions.close}
          </Button>
        }
      >
        <p className="m-0 font-semibold text-ink">{t.upgrade.notes}</p>
        <ul className="m-0 pl-5 list-disc text-[13px]">
          {result?.notes.map((n) => (
            <li key={n}>{n}</li>
          ))}
        </ul>
      </Dialog>
    </Card>
  );
}

function UpdateRow({
  text,
  button,
  disabled,
  onClick,
  primary,
}: {
  text: string;
  button: string;
  disabled: boolean;
  onClick: () => void;
  primary?: boolean;
}) {
  return (
    <div className="flex flex-wrap items-center gap-3 px-3.5 py-3 rounded-[10px] bg-accent-soft">
      <ArrowUpCircle size={18} aria-hidden="true" className="text-accent shrink-0" />
      <span className="flex-[1_1_260px] text-[13px] text-accent-text">{text}</span>
      <Button size="sm" variant={primary ? "primary" : "secondary"} className="!h-10" disabled={disabled} onClick={onClick}>
        {button}
      </Button>
    </div>
  );
}

function PlanRow({ mod, disableMissing }: { mod: ModCompat; disableMissing: boolean }) {
  const [text, tone] =
    mod.status === "ready"
      ? [t.upgrade.ready(mod.newVersion ?? ""), "text-ok-text"]
      : mod.status === "missing"
        ? [disableMissing ? t.upgrade.missingMod : t.upgrade.missingPlugin, disableMissing ? "text-err" : "text-warn"]
        : [t.upgrade.unknown, "text-muted"];
  return (
    <li className="flex flex-wrap items-baseline gap-x-2 text-[13px]">
      <span className="font-semibold text-ink">{mod.title}</span>
      <span className={tone}>{text}</span>
    </li>
  );
}
