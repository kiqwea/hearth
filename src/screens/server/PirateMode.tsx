// «Вход с пиратских клиентов» и «Защита ников паролем» в настройках сервера.
//
// Включение делает ядро: бэкап мира, плагин авторизации (AuthMe для Paper,
// EasyAuth для Fabric) по желанию, перенос вещей игроков на новые UUID,
// server.properties. Защиту ников можно включать и выключать отдельно:
// своей маленькой компании пароли при входе могут только мешать.

import { useState } from "react";
import { TriangleExclamationPoint } from "lucide-react";

import type { ServerSummary } from "../../bindings/ServerSummary";
import { Button, Dialog, Spinner, Switch } from "../../components/ui";
import { t } from "../../i18n";
import { api, isHearthError } from "../../lib/api";
import { isActive, isPaperLike } from "../../lib/format";
import { useHearth } from "../../lib/store";

type Ask = { kind: "enable" } | { kind: "disable" } | { kind: "noAuth"; reason: string };

export function PirateMode({ server, onChanged }: { server: ServerSummary; onChanged: () => void }) {
  const { meta, status } = server;
  const toast = useHearth((s) => s.toast);
  const toastError = useHearth((s) => s.toastError);
  const refreshServers = useHearth((s) => s.refreshServers);
  const [ask, setAsk] = useState<Ask | null>(null);
  const [protect, setProtect] = useState(true);
  const [working, setWorking] = useState<"pirate" | "protection" | null>(null);

  const active = isActive(status);
  const plugin = isPaperLike(meta.loader) ? "AuthMe Reloaded" : meta.loader === "fabric" ? "EasyAuth" : null;
  const protectedNow = !!meta.authPlugin;

  async function applyPirate(enable: boolean, withProtection: boolean) {
    setAsk(null);
    setWorking("pirate");
    try {
      const result = await api.setPirateMode(meta.id, enable, withProtection);
      await refreshServers();
      onChanged();
      toast("success", enable ? t.pirate.enabled(result.auth) : t.pirate.disabled);
    } catch (e) {
      if (enable && isHearthError(e) && e.kind === "authUnavailable") {
        setAsk({ kind: "noAuth", reason: e.message });
      } else {
        toastError(e);
      }
    } finally {
      setWorking(null);
    }
  }

  async function applyProtection(on: boolean) {
    setWorking("protection");
    try {
      await api.setNickProtection(meta.id, on);
      await refreshServers();
      toast("success", on ? t.protection.enabled : t.protection.disabled);
    } catch (e) {
      toastError(e);
    } finally {
      setWorking(null);
    }
  }

  let hint: string;
  if (working === "pirate") hint = t.pirate.working;
  else if (active) hint = t.pirate.stopFirst;
  else if (!meta.pirateMode) hint = t.pirate.offHint;
  else hint = t.pirate.onShort;

  return (
    <>
      <Row label={t.pirate.label} hint={hint}>
        {working === "pirate" ? (
          <Spinner className="!w-6 !h-6" />
        ) : (
          <Switch
            checked={meta.pirateMode}
            disabled={active || working !== null}
            label={t.pirate.label}
            onChange={(on) => {
              setProtect(true);
              setAsk({ kind: on ? "enable" : "disable" });
            }}
          />
        )}
      </Row>

      {meta.pirateMode &&
        (plugin ? (
          <Row
            label={t.protection.label}
            hint={
              working === "protection"
                ? t.protection.working
                : active
                  ? t.pirate.stopFirst
                  : protectedNow
                    ? t.protection.onHint
                    : t.protection.offHint
            }
            warn={!protectedNow && working !== "protection"}
          >
            {working === "protection" ? (
              <Spinner className="!w-6 !h-6" />
            ) : (
              <Switch
                checked={protectedNow}
                disabled={active || working !== null}
                label={t.protection.label}
                onChange={(on) => void applyProtection(on)}
              />
            )}
          </Row>
        ) : (
          <Row label={t.protection.label} hint={t.wizard.vanillaNoProtection} warn>
            <span />
          </Row>
        ))}

      <Dialog
        open={ask?.kind === "enable"}
        onClose={() => setAsk(null)}
        title={t.pirate.enableTitle}
        footer={
          <>
            <Button variant="ghost" onClick={() => setAsk(null)}>
              {t.actions.cancel}
            </Button>
            <Button variant={plugin ? "primary" : "danger"} onClick={() => void applyPirate(true, !!plugin && protect)}>
              {plugin ? t.pirate.enable : t.pirate.enableAnyway}
            </Button>
          </>
        }
      >
        {plugin ? (
          <label className="flex items-start gap-2.5 cursor-pointer">
            <input
              type="checkbox"
              checked={protect}
              onChange={(e) => setProtect(e.target.checked)}
              className="w-4 h-4 mt-1"
            />
            <span className="flex flex-col gap-1">
              <span className="text-ink font-semibold">{t.protection.label} ({plugin})</span>
              <span className="text-[13px]">{protect ? t.protection.onHint : t.protection.offHint}</span>
            </span>
          </label>
        ) : (
          <Warning>{t.pirate.vanillaWarning}</Warning>
        )}
        <p className="m-0">{t.pirate.uuidText}</p>
      </Dialog>

      <Dialog
        open={ask?.kind === "noAuth"}
        onClose={() => setAsk(null)}
        title={t.pirate.noAuthTitle}
        footer={
          <>
            <Button variant="ghost" onClick={() => setAsk(null)}>
              {t.actions.cancel}
            </Button>
            <Button variant="danger" onClick={() => void applyPirate(true, false)}>
              {t.pirate.enableAnyway}
            </Button>
          </>
        }
      >
        <p className="m-0">{ask?.kind === "noAuth" ? ask.reason : ""}</p>
        <Warning>{t.pirate.noAuthWarning}</Warning>
      </Dialog>

      <Dialog
        open={ask?.kind === "disable"}
        onClose={() => setAsk(null)}
        title={t.pirate.disableTitle}
        footer={
          <>
            <Button variant="ghost" onClick={() => setAsk(null)}>
              {t.actions.cancel}
            </Button>
            <Button variant="primary" onClick={() => void applyPirate(false, false)}>
              {t.pirate.disable}
            </Button>
          </>
        }
      >
        <p className="m-0">{t.pirate.disableText(protectedNow ? plugin : null)}</p>
      </Dialog>
    </>
  );
}

function Row({ label, hint, warn, children }: { label: string; hint: string; warn?: boolean; children: React.ReactNode }) {
  return (
    <div className="flex flex-wrap items-center gap-3 py-3.5">
      <span className="flex-[1_1_280px] flex flex-col gap-0.5">
        <span className="font-semibold">{label}</span>
        <span className={`text-[13px] ${warn ? "text-warn" : "text-muted"}`}>{hint}</span>
      </span>
      {children}
    </div>
  );
}

function Warning({ children }: { children: string }) {
  return (
    <span className="flex items-start gap-2.5 px-3.5 py-3 rounded-[10px] bg-err-bg text-err">
      <TriangleExclamationPoint size={18} className="shrink-0 mt-0.5" aria-hidden="true" />
      <span>{children}</span>
    </span>
  );
}
