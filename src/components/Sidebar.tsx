import { Flame, Plus, Settings } from "lucide-react";

import { t } from "../i18n";
import { avatarColors, initial, loaderName, statusText, statusTone } from "../lib/format";
import { useHearth } from "../lib/store";
import { StatusDot } from "./ui";

export function Sidebar() {
  const servers = useHearth((s) => s.servers);
  const view = useHearth((s) => s.view);
  const openServer = useHearth((s) => s.openServer);
  const navigate = useHearth((s) => s.navigate);
  const install = useHearth((s) => s.install);
  const openSettings = useHearth((s) => s.setAppSettingsOpen);

  const list = Object.values(servers).sort((a, b) => a.meta.createdAt.localeCompare(b.meta.createdAt));
  const selectedId = view.kind === "server" ? view.id : null;

  return (
    <aside className="w-[264px] shrink-0 bg-side border-r border-line px-3.5 py-5 flex flex-col gap-[18px] overflow-y-auto">
      <div className="flex items-center gap-2.5 px-2">
        <Flame size={26} strokeWidth={1.8} className="text-accent" aria-hidden="true" />
        <span className="text-xl font-bold tracking-tight">{t.app.name}</span>
      </div>

      <nav aria-label={t.app.servers} className="flex flex-col gap-1">
        {list.length > 0 && (
          <div className="text-xs font-semibold text-muted uppercase tracking-[0.06em] px-2 pb-1.5">{t.app.servers}</div>
        )}
        {list.map(({ meta, status }) => {
          const selected = meta.id === selectedId;
          const colors = avatarColors(meta.id);
          const progress = install[meta.id];
          let subtitle = `${loaderName(meta.loader)} ${meta.gameVersion} · ${statusText(status).toLowerCase()}`;
          if (status.state === "installing" && progress?.total && progress.downloaded !== null) {
            subtitle = `${t.status.installing} · ${Math.round((progress.downloaded / progress.total) * 100)} %`;
          }
          return (
            <button
              key={meta.id}
              type="button"
              aria-current={selected ? "page" : undefined}
              onClick={() => openServer(meta.id)}
              className={`flex items-center gap-3 p-2.5 rounded-[10px] text-left transition ${
                selected ? "bg-hover" : "hover:bg-[#211c18]"
              }`}
            >
              <span
                className="w-9 h-9 rounded-[9px] flex items-center justify-center font-bold shrink-0"
                style={{ background: colors.bg, color: colors.fg }}
              >
                {initial(meta.name)}
              </span>
              <span className="flex flex-col gap-0.5 min-w-0 flex-1">
                <span className="font-semibold leading-snug line-clamp-2 break-words" title={meta.name}>
                  {meta.name}
                </span>
                <span className={`text-xs truncate ${status.state === "installing" ? "text-link" : "text-muted"}`}>{subtitle}</span>
              </span>
              <StatusDot tone={statusTone(status)} />
            </button>
          );
        })}
      </nav>

      <div className="mt-auto flex flex-col gap-2">
        <button
          type="button"
          onClick={() => navigate({ kind: "wizard" })}
          className={`flex items-center justify-center gap-2 h-11 rounded-[10px] font-bold transition ${
            view.kind === "wizard"
              ? "border-2 border-accent bg-accent-soft text-accent-text"
              : "bg-accent text-accent-ink hover:brightness-110"
          }`}
        >
          <Plus size={18} strokeWidth={2.2} aria-hidden="true" />
          {t.app.newServer}
        </button>
        <button
          type="button"
          onClick={() => openSettings(true)}
          className="flex items-center gap-2.5 h-11 px-3 rounded-[10px] text-muted hover:text-ink hover:bg-[#211c18] transition"
        >
          <Settings size={18} strokeWidth={1.8} aria-hidden="true" />
          {t.app.appSettings}
        </button>
      </div>
    </aside>
  );
}
