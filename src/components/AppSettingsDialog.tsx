import { useEffect, useState } from "react";

import type { InstalledJava } from "../bindings/InstalledJava";
import { t } from "../i18n";
import { api, openExternal } from "../lib/api";
import { useHearth } from "../lib/store";
import { Button, Dialog, inputClass, Switch } from "./ui";

export function AppSettingsDialog() {
  const open = useHearth((s) => s.appSettingsOpen);
  const setOpen = useHearth((s) => s.setAppSettingsOpen);
  const config = useHearth((s) => s.config);
  const system = useHearth((s) => s.system);
  const setConfig = useHearth((s) => s.setConfig);
  const toast = useHearth((s) => s.toast);
  const toastError = useHearth((s) => s.toastError);
  const refreshServers = useHearth((s) => s.refreshServers);

  const [closeToTray, setCloseToTray] = useState(true);
  const [serversDir, setServersDir] = useState("");
  const [java, setJava] = useState<InstalledJava[]>([]);
  const [saving, setSaving] = useState(false);
  const [checking, setChecking] = useState(false);
  const checkUpdates = useHearth((s) => s.checkUpdates);
  const setUpdateOpen = useHearth((s) => s.setUpdateDialogOpen);

  useEffect(() => {
    if (!open || !config) return;
    setCloseToTray(config.closeToTray);
    setServersDir(config.serversDir ?? "");
    api.listJava().then(setJava, () => setJava([]));
  }, [open, config]);

  async function save() {
    if (!config) return;
    setSaving(true);
    try {
      const next = await api.setConfig({ ...config, closeToTray, serversDir: serversDir.trim() || null });
      setConfig(next);
      if ((next.serversDir ?? "") !== (config.serversDir ?? "")) await refreshServers();
      toast("success", t.settings.saved);
      setOpen(false);
    } catch (e) {
      toastError(e);
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog
      open={open}
      onClose={() => setOpen(false)}
      title={t.appSettings.title}
      wide
      footer={
        <>
          <Button variant="ghost" onClick={() => setOpen(false)}>
            {t.actions.cancel}
          </Button>
          <Button variant="primary" onClick={save} disabled={saving}>
            {t.actions.save}
          </Button>
        </>
      }
    >
      <div className="flex items-center gap-3">
        <span className="flex-1 flex flex-col gap-0.5">
          <span className="font-semibold text-ink">{t.appSettings.closeToTray}</span>
          <span className="text-[13px] text-muted">{t.appSettings.closeToTrayHint}</span>
        </span>
        <Switch checked={closeToTray} onChange={setCloseToTray} label={t.appSettings.closeToTray} />
      </div>

      <div className="flex flex-col gap-2 pt-2">
        <label htmlFor="app-servers-dir" className="font-semibold text-ink">
          {t.appSettings.serversDir}
        </label>
        <input
          id="app-servers-dir"
          value={serversDir}
          onChange={(e) => setServersDir(e.target.value)}
          placeholder={t.appSettings.serversDirPlaceholder}
          className={`${inputClass} font-mono text-[13px] !bg-bg`}
        />
        <span className="text-[13px] text-muted">{t.appSettings.serversDirHint}</span>
      </div>

      <PlayitSection />

      <div className="flex flex-col gap-2 pt-2">
        <span className="font-semibold text-ink">{t.appSettings.java}</span>
        {java.length === 0 ? (
          <span className="text-[13px] text-muted">{t.appSettings.noJava}</span>
        ) : (
          <ul className="m-0 p-0 list-none flex flex-wrap gap-2">
            {java.map((j) => (
              <li key={j.major} className="px-2.5 py-1 rounded-full bg-chip text-soft text-[13px]">
                Java {j.major} · {j.release}
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="pt-3 border-t border-line text-[13px] text-muted flex flex-col gap-1">
        <div className="flex flex-wrap items-center justify-between gap-2">
          {system && <span>{t.appSettings.version(system.appVersion)}</span>}
          <Button
            size="sm"
            variant="ghost"
            disabled={checking}
            onClick={async () => {
              setChecking(true);
              await checkUpdates(true);
              setChecking(false);
              if (useHearth.getState().update) {
                setOpen(false);
                setUpdateOpen(true);
              }
            }}
          >
            {t.update.check}
          </Button>
        </div>
        <span>{t.appSettings.disclaimer}</span>
      </div>
    </Dialog>
  );
}

/** Подключение к playit.gg: показать, открыть панель, отключить. */
function PlayitSection() {
  const playit = useHearth((s) => s.playit);
  const toast = useHearth((s) => s.toast);
  const toastError = useHearth((s) => s.toastError);
  const [confirm, setConfirm] = useState(false);
  const linked = playit?.linked ?? false;

  return (
    <div className="flex flex-col gap-2 pt-2">
      <span className="font-semibold text-ink">{t.playitSettings.title}</span>
      <div className="flex flex-wrap items-center gap-2.5">
        <span className="flex-[1_1_240px] text-[13px] text-muted">
          {linked ? t.playitSettings.linked : t.playitSettings.notLinked}
        </span>
        {linked && (
          <>
            <Button
              size="sm"
              variant="ghost"
              onClick={async () => {
                try {
                  await openExternal(await api.playitDashboardUrl());
                } catch (e) {
                  toastError(e);
                }
              }}
            >
              {t.access.dashboard}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setConfirm(true)}>
              {t.playitSettings.unlink}
            </Button>
          </>
        )}
      </div>
      {confirm && (
        <div role="alertdialog" className="flex flex-col gap-2.5 p-3.5 rounded-[10px] bg-raised">
          <span className="font-semibold text-ink">{t.playitSettings.unlinkTitle}</span>
          <span className="text-[13px]">{t.playitSettings.unlinkText}</span>
          <div className="flex flex-wrap justify-end gap-2">
            <Button size="sm" variant="ghost" onClick={() => setConfirm(false)}>
              {t.actions.cancel}
            </Button>
            <Button
              size="sm"
              variant="danger"
              onClick={async () => {
                setConfirm(false);
                try {
                  await api.playitUnlink();
                  toast("success", t.playitSettings.unlinked);
                } catch (e) {
                  toastError(e);
                }
              }}
            >
              {t.playitSettings.unlink}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
