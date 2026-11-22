// Переключатель «Вход с пиратских клиентов» в настройках сервера.
//
// Включение делает ядро: бэкап мира, плагин авторизации (AuthMe для Paper,
// EasyAuth для Fabric), перенос вещей игроков на новые UUID, server.properties.
// Здесь только объяснения и подтверждения: опасные последствия нужно показать заранее.

import { useState } from "react";
import { TriangleExclamationPoint } from "lucide-react";

import type { ServerSummary } from "../../bindings/ServerSummary";
import { Button, Dialog, Spinner, Switch } from "../../components/ui";
import { t } from "../../i18n";
import { api, isHearthError } from "../../lib/api";
import { isActive } from "../../lib/format";
import { useHearth } from "../../lib/store";

type Ask = { kind: "enable" } | { kind: "disable" } | { kind: "noAuth"; reason: string };

export function PirateMode({ server, onChanged }: { server: ServerSummary; onChanged: () => void }) {
  const { meta, status } = server;
  const toast = useHearth((s) => s.toast);
  const toastError = useHearth((s) => s.toastError);
  const refreshServers = useHearth((s) => s.refreshServers);
  const [ask, setAsk] = useState<Ask | null>(null);
  const [working, setWorking] = useState(false);

  const active = isActive(status);
  const plugin = meta.loader === "paper" ? "AuthMe Reloaded" : meta.loader === "fabric" ? "EasyAuth" : null;

  async function apply(enable: boolean, allowWithoutAuth: boolean) {
    setAsk(null);
    setWorking(true);
    try {
      const result = await api.setPirateMode(meta.id, enable, allowWithoutAuth);
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
      setWorking(false);
    }
  }

  let hint: string;
  if (working) hint = t.pirate.working;
  else if (active) hint = t.pirate.stopFirst;
  else if (!meta.pirateMode) hint = t.pirate.offHint;
  else if (meta.authPlugin) hint = t.pirate.onHint(plugin ?? "");
  else hint = t.pirate.onNoAuth;

  return (
    <div className="flex flex-wrap items-center gap-3 py-3.5">
      <span className="flex-[1_1_280px] flex flex-col gap-0.5">
        <span className="font-semibold">{t.pirate.label}</span>
        <span className={`text-[13px] ${meta.pirateMode && !meta.authPlugin && !working ? "text-err" : "text-muted"}`}>
          {hint}
        </span>
      </span>
      {working ? (
        <Spinner className="!w-6 !h-6" />
      ) : (
        <Switch
          checked={meta.pirateMode}
          disabled={active}
          label={t.pirate.label}
          onChange={(on) => setAsk({ kind: on ? "enable" : "disable" })}
        />
      )}

      <Dialog
        open={ask?.kind === "enable"}
        onClose={() => setAsk(null)}
        title={t.pirate.enableTitle}
        footer={
          <>
            <Button variant="ghost" onClick={() => setAsk(null)}>
              {t.actions.cancel}
            </Button>
            <Button variant={plugin ? "primary" : "danger"} onClick={() => void apply(true, !plugin)}>
              {plugin ? t.pirate.enable : t.pirate.enableAnyway}
            </Button>
          </>
        }
      >
        {plugin ? (
          <p className="m-0">{t.pirate.authText(plugin)}</p>
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
            <Button variant="danger" onClick={() => void apply(true, true)}>
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
            <Button variant="primary" onClick={() => void apply(false, false)}>
              {t.pirate.disable}
            </Button>
          </>
        }
      >
        <p className="m-0">{t.pirate.disableText(plugin)}</p>
      </Dialog>
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
