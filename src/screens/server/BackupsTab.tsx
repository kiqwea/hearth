// Вкладка «Бэкапы»: копии мира (вручную и по расписанию), восстановление,
// перенос мира из одиночной игры и скачивание мира в zip.
// Копии можно делать и на работающем сервере: ядро ставит автосохранение на
// паузу и сбрасывает мир на диск. Восстановление и замена мира — только на остановленном.

import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Download, FolderOpen, RotateCcw, Trash } from "lucide-react";

import type { AutoBackup } from "../../bindings/AutoBackup";
import type { BackupInfo } from "../../bindings/BackupInfo";
import type { SavedWorld } from "../../bindings/SavedWorld";
import type { ServerSummary } from "../../bindings/ServerSummary";
import { Button, Card, Dialog, IconButton, Spinner, Switch } from "../../components/ui";
import { t } from "../../i18n";
import { api } from "../../lib/api";
import { formatSize, formatWhen, isActive } from "../../lib/format";
import { useHearth } from "../../lib/store";

const selectClass = "h-10 px-3 rounded-[9px] border border-line-strong bg-bg text-ink w-[200px] max-w-full disabled:opacity-50";
const EVERY = [30, 60, 120, 360, 720, 1440];
const KEEP = [3, 5, 10, 20, 50];

export function BackupsTab({ server }: { server: ServerSummary }) {
  const { meta, status } = server;
  const toast = useHearth((s) => s.toast);
  const toastError = useHearth((s) => s.toastError);
  const refreshServers = useHearth((s) => s.refreshServers);
  const busy = useHearth((s) => s.backupBusy[meta.id] ?? false);
  const rev = useHearth((s) => s.backupsRev[meta.id] ?? 0);

  const [list, setList] = useState<BackupInfo[] | null>(null);
  const [making, setMaking] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [restoreAsk, setRestoreAsk] = useState<BackupInfo | null>(null);
  const [deleteAsk, setDeleteAsk] = useState<BackupInfo | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [working, setWorking] = useState(false);

  const active = isActive(status);
  const running = status.state === "running";
  // Копию на запущенном сервере можно сделать, только когда он полностью запустился.
  const canBackup = !busy && !making && !working && (running || !active);

  const load = useCallback(async () => {
    try {
      setList(await api.listBackups(meta.id));
    } catch (e) {
      toastError(e);
    }
  }, [meta.id, toastError]);

  useEffect(() => {
    void load();
  }, [load, rev]);

  // Копия могла начаться до открытия вкладки (автобэкап).
  useEffect(() => {
    void api.backupInProgress(meta.id).then(
      (inProgress) => useHearth.setState((st) => ({ backupBusy: { ...st.backupBusy, [meta.id]: inProgress } })),
      () => {},
    );
  }, [meta.id]);

  async function makeBackup() {
    setMaking(true);
    try {
      const info = await api.createBackup(meta.id);
      toast(info ? "success" : "info", info ? t.backups.made : t.backups.noWorld);
      await load();
    } catch (e) {
      toastError(e);
    } finally {
      setMaking(false);
    }
  }

  async function restore(b: BackupInfo) {
    setRestoreAsk(null);
    setWorking(true);
    try {
      const safety = await api.restoreBackup(meta.id, b.file);
      toast("success", safety ? t.backups.safety(t.backups.restored) : t.backups.restored);
      await load();
    } catch (e) {
      toastError(e);
    } finally {
      setWorking(false);
    }
  }

  async function remove(b: BackupInfo) {
    setDeleteAsk(null);
    try {
      await api.deleteBackup(meta.id, b.file);
      toast("success", t.backups.deleted);
      await load();
    } catch (e) {
      toastError(e);
    }
  }

  async function saveCopyAs(b: BackupInfo) {
    try {
      const dest = await api.pickSaveZip(`${meta.name} ${b.file}`);
      if (!dest) return;
      await api.exportBackup(meta.id, b.file, dest);
      toast("success", t.backups.exported);
    } catch (e) {
      toastError(e);
    }
  }

  async function exportWorld() {
    try {
      const dest = await api.pickSaveZip(`${meta.name}.zip`);
      if (!dest) return;
      setExporting(true);
      await api.exportWorld(meta.id, dest);
      toast("success", t.backups.exported);
    } catch (e) {
      toastError(e);
    } finally {
      setExporting(false);
    }
  }

  async function setAuto(patch: Partial<AutoBackup>) {
    try {
      await api.updateServer(meta.id, { autoBackup: { ...meta.autoBackup, ...patch } });
      await refreshServers();
    } catch (e) {
      toastError(e);
    }
  }

  const total = list?.reduce((sum, b) => sum + b.sizeBytes, 0) ?? 0;

  return (
    <div className="flex flex-col gap-[18px] max-w-[860px]">
      <Card label={t.backups.title} className="px-5 py-1.5">
        <div className="flex flex-wrap items-center gap-3 mt-3.5 mb-1.5">
          <h2 className="m-0 text-base font-semibold">{t.backups.title}</h2>
          {list && list.length > 0 && <span className="text-muted text-[13px]">{t.backups.summary(list.length, formatSize(total))}</span>}
          <span className="flex-1" />
          <IconButton label={t.backups.openFolder} onClick={() => void api.openBackupsFolder(meta.id).catch(toastError)}>
            <FolderOpen size={17} aria-hidden="true" />
          </IconButton>
          <Button size="sm" variant="primary" className="!h-10" disabled={!canBackup} onClick={() => void makeBackup()}>
            {t.backups.makeNow}
          </Button>
        </div>

        {(busy || making) && (
          <p className="m-0 mb-2 flex items-center gap-2.5 text-accent-text" role="status">
            <Spinner className="!w-4 !h-4 !border-2" />
            {t.backups.making}
          </p>
        )}
        {active && <p className="mt-0 mb-2 text-[13px] text-muted">{t.backups.stopToRestore}</p>}

        {list === null ? (
          <div className="py-6 flex justify-center">
            <Spinner />
          </div>
        ) : list.length === 0 ? (
          <p className="mt-1 mb-3.5 text-muted">{t.backups.empty}</p>
        ) : (
          <ul className="m-0 p-0 list-none flex flex-col [&>*:not(:last-child)]:border-b [&>*:not(:last-child)]:border-[#262119]">
            {list.map((b) => (
              <li key={b.file} className="flex flex-wrap items-center gap-3 py-3">
                <span className="flex-[1_1_240px] min-w-0 flex flex-col gap-0.5">
                  <span className="font-semibold first-letter:uppercase">{formatWhen(b.createdAt)}</span>
                  <span className="text-[13px] text-muted">
                    {t.backups.kind[b.kind]} · {formatSize(b.sizeBytes)}
                  </span>
                </span>
                <span className="flex flex-wrap items-center gap-1.5">
                  <Button size="sm" variant="ghost" disabled={active || working || busy} onClick={() => setRestoreAsk(b)}>
                    <RotateCcw size={15} aria-hidden="true" />
                    {t.backups.restore}
                  </Button>
                  <IconButton label={t.backups.saveAs} className="!w-9 !h-9" onClick={() => void saveCopyAs(b)}>
                    <Download size={16} aria-hidden="true" />
                  </IconButton>
                  <IconButton label={t.backups.delete} className="!w-9 !h-9 hover:!text-err" onClick={() => setDeleteAsk(b)}>
                    <Trash size={16} aria-hidden="true" />
                  </IconButton>
                </span>
              </li>
            ))}
          </ul>
        )}
      </Card>

      <Card label={t.backups.auto} className="px-5 py-1.5">
        <div className="flex flex-col [&>*:not(:last-child)]:border-b [&>*:not(:last-child)]:border-[#262119]">
          <Row label={t.backups.auto} hint={t.backups.autoHint}>
            <Switch checked={meta.autoBackup.enabled} onChange={(enabled) => void setAuto({ enabled })} label={t.backups.auto} />
          </Row>
          <Row label={t.backups.autoEvery} htmlFor="auto-every">
            <select
              id="auto-every"
              className={selectClass}
              disabled={!meta.autoBackup.enabled}
              value={meta.autoBackup.everyMinutes}
              onChange={(e) => void setAuto({ everyMinutes: Number(e.target.value) })}
            >
              {withCurrent(EVERY, meta.autoBackup.everyMinutes).map((m) => (
                <option key={m} value={m}>
                  {t.backups.every(m)}
                </option>
              ))}
            </select>
          </Row>
          <Row label={t.backups.autoKeep} htmlFor="auto-keep">
            <select
              id="auto-keep"
              className={selectClass}
              disabled={!meta.autoBackup.enabled}
              value={meta.autoBackup.keep}
              onChange={(e) => void setAuto({ keep: Number(e.target.value) })}
            >
              {withCurrent(KEEP, meta.autoBackup.keep).map((n) => (
                <option key={n} value={n}>
                  {t.backups.keep(n)}
                </option>
              ))}
            </select>
          </Row>
        </div>
      </Card>

      <Card label={t.backups.world} className="px-5 py-1.5">
        <h2 className="mt-3.5 mb-1.5 text-base font-semibold">{t.backups.world}</h2>
        <div className="flex flex-col [&>*:not(:last-child)]:border-b [&>*:not(:last-child)]:border-[#262119]">
          <Row label={t.backups.importTitle} hint={t.backups.importHint}>
            <Button disabled={active || working || busy} onClick={() => setImportOpen(true)}>
              {working ? t.backups.replacing : t.backups.importButton}
            </Button>
          </Row>
          <Row label={t.backups.exportTitle} hint={t.backups.exportHint}>
            <Button disabled={exporting || busy || (active && !running)} onClick={() => void exportWorld()}>
              {exporting ? (
                <>
                  <Spinner className="!w-4 !h-4 !border-2" />
                  {t.backups.exporting}
                </>
              ) : (
                t.backups.exportButton
              )}
            </Button>
          </Row>
        </div>
      </Card>

      <Dialog
        open={restoreAsk !== null}
        onClose={() => setRestoreAsk(null)}
        title={t.backups.restoreTitle}
        footer={
          <>
            <Button variant="ghost" onClick={() => setRestoreAsk(null)}>
              {t.actions.cancel}
            </Button>
            <Button variant="primary" onClick={() => restoreAsk && void restore(restoreAsk)}>
              {t.backups.restore}
            </Button>
          </>
        }
      >
        <p className="m-0">{restoreAsk && t.backups.restoreText(formatWhen(restoreAsk.createdAt))}</p>
      </Dialog>

      <Dialog
        open={deleteAsk !== null}
        onClose={() => setDeleteAsk(null)}
        title={t.backups.deleteTitle}
        footer={
          <>
            <Button variant="ghost" onClick={() => setDeleteAsk(null)}>
              {t.actions.cancel}
            </Button>
            <Button variant="danger" onClick={() => deleteAsk && void remove(deleteAsk)}>
              <Trash size={16} aria-hidden="true" />
              {t.actions.delete}
            </Button>
          </>
        }
      >
        <p className="m-0">{deleteAsk && t.backups.deleteText(formatWhen(deleteAsk.createdAt))}</p>
      </Dialog>

      <ImportDialog
        open={importOpen}
        version={meta.gameVersion}
        onClose={() => setImportOpen(false)}
        onImport={async (source) => {
          setImportOpen(false);
          setWorking(true);
          try {
            const safety = await api.importWorld(meta.id, source);
            toast("success", safety ? t.backups.safety(t.backups.imported) : t.backups.imported);
            await load();
          } catch (e) {
            toastError(e);
          } finally {
            setWorking(false);
          }
        }}
      />
    </div>
  );
}

/** Значение из файла настроек могли поставить вручную: показываем его тоже. */
function withCurrent(options: number[], current: number): number[] {
  return options.includes(current) ? options : [...options, current].sort((a, b) => a - b);
}

function ImportDialog({
  open,
  version,
  onClose,
  onImport,
}: {
  open: boolean;
  version: string;
  onClose: () => void;
  onImport: (source: string) => Promise<void>;
}) {
  const toastError = useHearth((s) => s.toastError);
  const [worlds, setWorlds] = useState<SavedWorld[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [custom, setCustom] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setSelected(null);
    setCustom(null);
    setWorlds(null);
    api.listSavedWorlds().then(setWorlds, (e) => {
      setWorlds([]);
      toastError(e);
    });
  }, [open, toastError]);

  async function pick(kind: "zip" | "folder") {
    try {
      const path = kind === "zip" ? await api.pickZip() : await api.pickFolder();
      if (path) {
        setCustom(path);
        setSelected(path);
      }
    } catch (e) {
      toastError(e);
    }
  }

  return (
    <Dialog
      open={open}
      onClose={onClose}
      wide
      title={t.backups.importDialog}
      footer={
        <>
          <Button variant="ghost" size="sm" className="!h-10" onClick={() => void pick("zip")}>
            {t.backups.pickZip}
          </Button>
          <Button variant="ghost" size="sm" className="!h-10" onClick={() => void pick("folder")}>
            {t.backups.pickFolder}
          </Button>
          <span className="flex-1" />
          <Button variant="ghost" onClick={onClose}>
            {t.actions.cancel}
          </Button>
          <Button variant="primary" disabled={!selected} onClick={() => selected && void onImport(selected)}>
            {t.backups.replace}
          </Button>
        </>
      }
    >
      <p className="m-0 text-[13px]">{t.backups.importDialogText(version)}</p>
      <div role="radiogroup" aria-label={t.backups.importDialog} className="flex flex-col gap-1.5 max-h-[340px] overflow-y-auto -mx-1 px-1">
        {custom && (
          <Choice checked={selected === custom} onSelect={() => setSelected(custom)}>
            <span className="font-semibold">{t.backups.picked}</span>
            <span className="text-[13px] text-muted break-all">{custom}</span>
          </Choice>
        )}
        {worlds === null ? (
          <p className="m-0 py-3 flex items-center gap-2.5 text-muted">
            <Spinner className="!w-4 !h-4 !border-2" />
            {t.backups.searching}
          </p>
        ) : worlds.length === 0 ? (
          !custom && <p className="m-0 py-2 text-muted">{t.backups.noSaves}</p>
        ) : (
          worlds.map((w) => (
            <Choice key={w.path} checked={selected === w.path} onSelect={() => setSelected(w.path)}>
              <span className="font-semibold break-words">{w.name}</span>
              <span className="text-[13px] text-muted">
                {[w.source, w.lastPlayed ? t.backups.playedAt(formatWhen(w.lastPlayed)) : null, formatSize(w.sizeBytes)]
                  .filter(Boolean)
                  .join(" · ")}
              </span>
            </Choice>
          ))
        )}
      </div>
    </Dialog>
  );
}

function Choice({ checked, onSelect, children }: { checked: boolean; onSelect: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={checked}
      onClick={onSelect}
      className={`text-left flex items-center gap-3 px-3.5 py-2.5 rounded-[10px] border transition ${
        checked ? "border-accent bg-accent-soft" : "border-line-strong hover:bg-hover"
      }`}
    >
      <span
        aria-hidden="true"
        className={`w-4 h-4 rounded-full border-2 shrink-0 ${checked ? "border-accent bg-accent shadow-[inset_0_0_0_3px_var(--color-card)]" : "border-line-strong"}`}
      />
      <span className="min-w-0 flex flex-col gap-0.5 text-ink">{children}</span>
    </button>
  );
}

function Row({ label, hint, htmlFor, children }: { label: string; hint?: string; htmlFor?: string; children: ReactNode }) {
  return (
    <div className="flex flex-wrap items-center gap-3 py-3.5">
      <span className="flex-[1_1_280px] flex flex-col gap-0.5">
        {htmlFor ? (
          <label htmlFor={htmlFor} className="font-semibold">
            {label}
          </label>
        ) : (
          <span className="font-semibold">{label}</span>
        )}
        {hint && <span className="text-[13px] text-muted">{hint}</span>}
      </span>
      {children}
    </div>
  );
}
