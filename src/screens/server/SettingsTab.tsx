import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { Info, Trash } from "lucide-react";

import type { ServerSummary } from "../../bindings/ServerSummary";
import { Button, Card, Dialog, Spinner, Switch } from "../../components/ui";
import { t } from "../../i18n";
import { api } from "../../lib/api";
import { gb, isActive } from "../../lib/format";
import { HANDLED_ELSEWHERE, KNOWN_KEYS, PROPERTY_GROUPS, type PropertyDef } from "../../lib/properties";
import { useHearth } from "../../lib/store";
import { PirateMode } from "./PirateMode";

const fieldClass = "h-10 px-3 rounded-[9px] border border-line-strong bg-bg text-ink placeholder:text-faint";

export function SettingsTab({ server }: { server: ServerSummary }) {
  const { meta, status } = server;
  const system = useHearth((s) => s.system);
  const toast = useHearth((s) => s.toast);
  const toastError = useHearth((s) => s.toastError);
  const refreshServers = useHearth((s) => s.refreshServers);
  const removeServer = useHearth((s) => s.removeServer);

  const [original, setOriginal] = useState<Record<string, string> | null>(null);
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [name, setName] = useState(meta.name);
  const [memoryMb, setMemoryMb] = useState(meta.memoryMb);
  const [saving, setSaving] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleteTunnel, setDeleteTunnel] = useState(true);

  const load = useCallback(async () => {
    try {
      const entries = await api.getProperties(meta.id);
      const map = Object.fromEntries(entries.map((e) => [e.key, e.value]));
      setOriginal(map);
      setDraft(map);
    } catch (e) {
      toastError(e);
    }
  }, [meta.id, toastError]);

  useEffect(() => {
    void load();
  }, [load]);

  // Пока сервер ни разу не запускался, в файле только наши параметры: показываем все.
  // После первого запуска Minecraft дописывает полный список, и мы показываем
  // только то, что поддерживает эта версия игры.
  const startedOnce = original !== null && "level-name" in original;
  const groups = useMemo(
    () =>
      PROPERTY_GROUPS.map((g) => ({
        ...g,
        items: g.items.filter((item) => !startedOnce || (original !== null && item.key in original)),
      })).filter((g) => g.items.length > 0),
    [original, startedOnce],
  );

  const value = (def: PropertyDef) => draft[def.key] ?? def.default;
  const set = (key: string, v: string) => setDraft((d) => ({ ...d, [key]: v }));

  const changedKeys = original
    ? Object.keys(draft).filter((k) => draft[k] !== (original[k] ?? PROPERTY_GROUPS.flatMap((g) => g.items).find((i) => i.key === k)?.default))
    : [];
  const metaChanged = name.trim() !== meta.name || memoryMb !== meta.memoryMb;
  const changes = changedKeys.length + (name.trim() !== meta.name ? 1 : 0) + (memoryMb !== meta.memoryMb ? 1 : 0);

  async function save() {
    setSaving(true);
    try {
      if (changedKeys.length) await api.setProperties(meta.id, changedKeys.map((key) => ({ key, value: draft[key] })));
      if (metaChanged) {
        await api.updateServer(meta.id, {
          name: name.trim() !== meta.name ? name.trim() : null,
          memoryMb: memoryMb !== meta.memoryMb ? memoryMb : null,
        });
      }
      await Promise.all([refreshServers(), load()]);
      toast("success", t.settings.saved);
    } catch (e) {
      toastError(e);
    } finally {
      setSaving(false);
    }
  }

  function reset() {
    if (original) setDraft(original);
    setName(meta.name);
    setMemoryMb(meta.memoryMb);
  }

  async function remove() {
    setConfirmDelete(false);
    try {
      await api.deleteServer(meta.id, deleteTunnel);
      removeServer(meta.id);
    } catch (e) {
      toastError(e);
    }
  }

  if (!original) {
    return (
      <div className="py-10 flex justify-center">
        <Spinner />
      </div>
    );
  }

  const totalGb = system ? Math.round(system.totalMemoryMb / 1024) : 16;
  const maxGb = Math.max(2, Math.min(32, Math.floor(totalGb * 0.75)));
  const active = isActive(status);
  const otherKeys = Object.keys(draft)
    .filter((k) => !KNOWN_KEYS.has(k) && !HANDLED_ELSEWHERE.has(k))
    .sort();

  return (
    <div className="flex flex-col gap-[18px] max-w-[860px]">
      {active && (
        <div className="flex items-center gap-2.5 px-4 py-3 rounded-xl bg-accent-soft text-accent-text">
          <Info size={18} className="shrink-0" aria-hidden="true" />
          {t.settings.restartNotice}
        </div>
      )}

      <Group title={t.settings.general}>
        <Row label={t.settings.name} htmlFor="set-name">
          <input id="set-name" value={name} maxLength={64} onChange={(e) => setName(e.target.value)} className={`${fieldClass} flex-[1_1_260px]`} />
        </Row>
        <Row label={t.settings.motd} hint={t.settings.motdHint} htmlFor="set-motd">
          <input
            id="set-motd"
            value={draft.motd ?? ""}
            onChange={(e) => set("motd", e.target.value)}
            className={`${fieldClass} flex-[1_1_260px]`}
          />
        </Row>
        <Row label={t.settings.memory(gb(memoryMb))} hint={t.settings.memoryHint(totalGb)} htmlFor="set-ram">
          <input
            id="set-ram"
            type="range"
            min={1}
            max={maxGb}
            value={Math.round(memoryMb / 1024)}
            onChange={(e) => setMemoryMb(Number(e.target.value) * 1024)}
            className="flex-[1_1_260px]"
          />
        </Row>
      </Group>

      {groups.map((g) => (
        <Group key={g.title} title={g.title}>
          {g.items.map((def) => (
            <PropertyRow key={def.key} def={def} value={value(def)} onChange={(v) => set(def.key, v)} />
          ))}
          {g.title === "Доступ" && <PirateMode server={server} onChanged={() => void load()} />}
        </Group>
      ))}

      {otherKeys.length > 0 && (
        <details className="bg-card border border-line rounded-[14px] px-5 py-4">
          <summary className="font-semibold">{t.settings.allProperties}</summary>
          <p className="mt-2 mb-3 text-[13px] text-muted">{t.settings.allPropertiesHint}</p>
          <div className="flex flex-col gap-2">
            {otherKeys.map((key) => (
              <div key={key} className="flex flex-wrap items-center gap-3">
                <label htmlFor={`raw-${key}`} className="flex-[1_1_240px] font-mono text-[13px] text-soft break-all">
                  {key}
                </label>
                <input
                  id={`raw-${key}`}
                  value={draft[key]}
                  onChange={(e) => set(key, e.target.value)}
                  className={`${fieldClass} flex-[1_1_260px] font-mono text-[13px]`}
                />
              </div>
            ))}
          </div>
        </details>
      )}

      <Group title={t.settings.danger}>
        <Row label={t.settings.deleteServer} hint={active ? t.settings.stopFirst : t.settings.dangerText}>
          <Button variant="danger" disabled={active} onClick={() => setConfirmDelete(true)}>
            <Trash size={16} aria-hidden="true" />
            {t.actions.delete}
          </Button>
        </Row>
      </Group>

      <footer className="sticky bottom-0 flex flex-wrap items-center justify-end gap-2.5 py-3.5 border-t border-line bg-bg">
        <span className="flex-[1_1_200px] text-muted text-[13px]">{t.settings.changed(changes)}</span>
        <Button variant="ghost" onClick={reset} disabled={changes === 0 || saving}>
          {t.actions.cancel}
        </Button>
        <Button variant="primary" onClick={save} disabled={changes === 0 || saving || !name.trim()}>
          {t.actions.save}
        </Button>
      </footer>

      <Dialog
        open={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        title={t.settings.deleteConfirmTitle}
        footer={
          <>
            <Button variant="ghost" onClick={() => setConfirmDelete(false)}>
              {t.actions.cancel}
            </Button>
            <Button variant="danger" onClick={remove}>
              <Trash size={16} aria-hidden="true" />
              {t.actions.delete}
            </Button>
          </>
        }
      >
        <p className="m-0">{t.settings.deleteConfirmText(meta.name)}</p>
        {meta.playit && (
          <label className="flex items-center gap-2.5 cursor-pointer">
            <input
              type="checkbox"
              checked={deleteTunnel}
              onChange={(e) => setDeleteTunnel(e.target.checked)}
              className="w-4 h-4"
            />
            <span>{t.access.deleteTunnel}</span>
          </label>
        )}
      </Dialog>
    </div>
  );
}

function Group({ title, children }: { title: string; children: ReactNode }) {
  return (
    <Card label={title} className="px-5 py-1.5">
      <h2 className="mt-3.5 mb-1.5 text-base font-semibold">{title}</h2>
      <div className="flex flex-col [&>*:not(:last-child)]:border-b [&>*:not(:last-child)]:border-[#262119]">{children}</div>
    </Card>
  );
}

function Row({
  label,
  hint,
  htmlFor,
  children,
}: {
  label: string;
  hint?: string;
  htmlFor?: string;
  children: ReactNode;
}) {
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

function PropertyRow({ def, value, onChange }: { def: PropertyDef; value: string; onChange: (v: string) => void }) {
  const id = `prop-${def.key}`;
  const kind = def.kind;
  switch (kind.type) {
    case "bool":
      return (
        <Row label={def.label} hint={def.hint}>
          <Switch checked={value === "true"} onChange={(v) => onChange(String(v))} label={def.label} />
        </Row>
      );
    case "int":
      return (
        <Row label={def.label} hint={def.hint} htmlFor={id}>
          <input
            id={id}
            type="number"
            min={kind.min}
            max={kind.max}
            value={value}
            onChange={(e) => onChange(e.target.value)}
            onBlur={(e) => {
              const n = Number(e.target.value);
              const clamped = Number.isFinite(n) ? Math.max(kind.min, Math.min(kind.max, Math.round(n))) : Number(def.default);
              onChange(String(clamped));
            }}
            className={`${fieldClass} w-[140px] ${def.key === "server-port" ? "font-mono" : ""}`}
          />
        </Row>
      );
    case "range":
      return (
        <Row label={`${def.label}: ${value} ${kind.unit}`} hint={def.hint} htmlFor={id}>
          <input
            id={id}
            type="range"
            min={kind.min}
            max={kind.max}
            value={value}
            onChange={(e) => onChange(e.target.value)}
            className="flex-[1_1_260px]"
          />
        </Row>
      );
    case "select": {
      const known = kind.options.some((o) => o.value === value);
      return (
        <Row label={def.label} hint={def.hint} htmlFor={id}>
          <select id={id} value={value} onChange={(e) => onChange(e.target.value)} className={`${fieldClass} w-[260px] max-w-full`}>
            {!known && <option value={value}>{value}</option>}
            {kind.options.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </Row>
      );
    }
    case "text":
      return (
        <Row label={def.label} hint={def.hint} htmlFor={id}>
          <input
            id={id}
            value={value}
            placeholder={kind.placeholder}
            onChange={(e) => onChange(e.target.value)}
            className={`${fieldClass} flex-[1_1_260px]`}
          />
        </Row>
      );
  }
}
