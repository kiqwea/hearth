// «Сборка для друзей» (Fabric): моды, нужные игрокам, одним файлом.
// .mrpack открывается в Prism Launcher и Modrinth App (лаунчер сам скачает
// моды и Fabric), zip — для TLauncher и других: папка mods и инструкция.

import { useEffect, useState } from "react";
import { Package } from "lucide-react";

import type { PackFormat } from "../../bindings/PackFormat";
import type { PackOptions } from "../../bindings/PackOptions";
import type { PackSummary } from "../../bindings/PackSummary";
import type { ServerSummary } from "../../bindings/ServerSummary";
import { Button, Card, Spinner } from "../../components/ui";
import { t } from "../../i18n";
import { api, errorMessage } from "../../lib/api";
import { useHearth } from "../../lib/store";

export function FriendPack({ server, modsKey }: { server: ServerSummary; modsKey: string }) {
  const { meta } = server;
  const toast = useHearth((s) => s.toast);
  const toastError = useHearth((s) => s.toastError);
  const [options, setOptions] = useState<PackOptions>({ optional: true, optifine: true });
  const [summary, setSummary] = useState<PackSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [building, setBuilding] = useState<PackFormat | null>(null);

  // Пересчитываем состав при смене галочек и модов сервера.
  useEffect(() => {
    let stale = false;
    setSummary(null);
    setError(null);
    api.packPreview(meta.id, options).then(
      (s) => !stale && setSummary(s),
      (e) => !stale && setError(errorMessage(e)),
    );
    return () => {
      stale = true;
    };
  }, [meta.id, options, modsKey]);

  async function exportPack(format: PackFormat) {
    try {
      const mrpack = format === "mrpack";
      const dest = await api.pickSave(
        `${meta.name} ${meta.gameVersion}.${mrpack ? "mrpack" : "zip"}`,
        mrpack ? "mrpack" : "zip",
        mrpack ? t.pack.mrpackLabel : t.pack.zipLabel,
      );
      if (!dest) return;
      setBuilding(format);
      await api.exportPack(meta.id, options, format, dest);
      toast("success", t.pack.saved);
    } catch (e) {
      toastError(e);
    } finally {
      setBuilding(null);
    }
  }

  const empty = summary !== null && summary.mods.length === 0;

  return (
    <Card label={t.pack.title} className="px-5 py-4 flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-4">
        <span aria-hidden="true" className="w-10 h-10 rounded-lg bg-accent-soft text-accent flex items-center justify-center shrink-0">
          <Package size={20} />
        </span>
        <span className="flex-[1_1_300px] flex flex-col gap-0.5">
          <h2 className="m-0 text-base font-semibold">{t.pack.title}</h2>
          <span className="text-[13px] text-muted">{t.pack.hint}</span>
        </span>
      </div>

      <div className="flex flex-col gap-2">
        <Check
          checked={options.optional}
          onChange={(optional) => setOptions((o) => ({ ...o, optional }))}
          label={t.pack.optional(summary?.optionalAvailable ?? 0)}
          hint={t.pack.optionalHint}
        />
        <Check
          checked={options.optifine}
          onChange={(optifine) => setOptions((o) => ({ ...o, optifine }))}
          label={t.pack.optifine}
          hint={t.pack.optifineHint}
        />
      </div>

      {error ? (
        <p className="m-0 text-[13px] text-err">{error}</p>
      ) : summary === null ? (
        <p className="m-0 flex items-center gap-2.5 text-[13px] text-muted">
          <Spinner className="!w-4 !h-4 !border-2" />
          {t.pack.counting}
        </p>
      ) : empty ? (
        <p className="m-0 text-[13px] text-muted">{t.pack.empty}</p>
      ) : (
        <div className="flex flex-col gap-1 text-[13px]">
          <span className="text-soft">
            <span className="font-semibold text-ink">{t.pack.contents(summary.mods.length)}:</span> {summary.mods.join(", ")}
          </span>
          {summary.address && <span className="text-muted">{t.pack.address(summary.address)}</span>}
          {summary.skipped.map((s) => (
            <span key={s} className="text-warn">
              {s}
            </span>
          ))}
        </div>
      )}

      <div className="flex flex-wrap gap-2.5">
        <ExportButton
          label={t.pack.mrpack}
          hint={t.pack.mrpackHint}
          busy={building === "mrpack"}
          disabled={building !== null || !summary || empty}
          onClick={() => void exportPack("mrpack")}
          primary
        />
        <ExportButton
          label={t.pack.zip}
          hint={t.pack.zipHint}
          busy={building === "zip"}
          disabled={building !== null || !summary || empty}
          onClick={() => void exportPack("zip")}
        />
      </div>
    </Card>
  );
}

function Check({ checked, onChange, label, hint }: { checked: boolean; onChange: (v: boolean) => void; label: string; hint: string }) {
  return (
    <label className="flex items-start gap-2.5 cursor-pointer">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} className="w-4 h-4 mt-[3px] shrink-0 accent-[var(--color-accent)]" />
      <span className="flex flex-col">
        <span>{label}</span>
        <span className="text-[13px] text-muted">{hint}</span>
      </span>
    </label>
  );
}

function ExportButton({
  label,
  hint,
  busy,
  disabled,
  onClick,
  primary,
}: {
  label: string;
  hint: string;
  busy: boolean;
  disabled: boolean;
  onClick: () => void;
  primary?: boolean;
}) {
  return (
    <Button variant={primary ? "primary" : "secondary"} disabled={disabled} onClick={onClick} className="!h-auto py-2 flex-col !items-start !gap-0">
      <span className="flex items-center gap-2">
        {busy && <Spinner className="!w-4 !h-4 !border-2" />}
        {busy ? t.pack.building : label}
      </span>
      <span className="text-[12px] font-normal opacity-80">{hint}</span>
    </Button>
  );
}
