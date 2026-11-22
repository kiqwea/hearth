import { useEffect, useState } from "react";

import type { InstalledJava } from "../bindings/InstalledJava";
import { t } from "../i18n";
import { api } from "../lib/api";
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
        {system && <span>{t.appSettings.version(system.appVersion)}</span>}
        <span>{t.appSettings.disclaimer}</span>
      </div>
    </Dialog>
  );
}
