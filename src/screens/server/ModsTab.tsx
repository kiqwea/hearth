// Вкладка «Плагины» (Paper, Purpur) или «Моды» (Fabric): что стоит на сервере
// и поиск на Modrinth. Hearth сам выбирает файл под ядро и версию игры и ставит
// обязательные зависимости. У модов Fabric видно, нужно ли их ставить игрокам.

import { useCallback, useEffect, useRef, useState } from "react";
import { FolderOpen, Search, Trash } from "lucide-react";

import type { ClientSide } from "../../bindings/ClientSide";
import type { InstalledMod } from "../../bindings/InstalledMod";
import type { ModHit } from "../../bindings/ModHit";
import type { ModList } from "../../bindings/ModList";
import type { ServerSummary } from "../../bindings/ServerSummary";
import { Button, Card, Dialog, IconButton, inputClass, Spinner } from "../../components/ui";
import { t } from "../../i18n";
import { api, errorMessage, openExternal } from "../../lib/api";
import { formatSize, isActive, isPaperLike } from "../../lib/format";
import { useHearth } from "../../lib/store";
import { FriendPack } from "./FriendPack";
import { PregenCard, PresetCard } from "./ServerBoost";

/** Chunky на Modrinth: нужен для предгенерации мира. */
const CHUNKY = "fALzjamp";

const SIDE_TONE: Record<ClientSide, string> = {
  required: "bg-warn-bg text-warn",
  optional: "bg-chip text-soft",
  serverOnly: "bg-chip text-soft",
  unknown: "",
};

export function ModsTab({ server }: { server: ServerSummary }) {
  const { meta, status } = server;
  const toast = useHearth((s) => s.toast);
  const toastError = useHearth((s) => s.toastError);
  const plugins = isPaperLike(meta.loader);
  const active = isActive(status);

  const [list, setList] = useState<ModList | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [removeAsk, setRemoveAsk] = useState<InstalledMod | null>(null);

  const load = useCallback(async () => {
    try {
      setList(await api.listMods(meta.id));
    } catch (e) {
      toastError(e);
    }
  }, [meta.id, toastError]);

  useEffect(() => {
    void load();
  }, [load]);

  const installedProjects = new Set(list?.items.map((m) => m.projectId).filter(Boolean));
  const updates = list?.items.filter((m) => m.update) ?? [];

  async function install(project: string) {
    setBusy(project);
    try {
      const files = await api.installMod(meta.id, project);
      toast("success", active ? `${t.mods.installedOk(files)}. ${t.mods.restartToApply}` : t.mods.installedOk(files));
      await load();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  }

  async function updateAll() {
    setBusy("update-all");
    try {
      const files = await api.updateMods(meta.id);
      toast("success", t.mods.updated(files.length));
      await load();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  }

  async function remove(m: InstalledMod) {
    setRemoveAsk(null);
    setBusy(m.file);
    try {
      await api.removeMod(meta.id, m.file);
      toast("success", t.mods.removed);
      await load();
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  }

  const modrinthUrl = (slug: string | null, projectId: string) =>
    `https://modrinth.com/${plugins ? "plugin" : "mod"}/${slug ?? projectId}`;

  return (
    <div className="flex flex-col gap-[18px] max-w-[860px]">
      {active && <p className="m-0 px-4 py-3 rounded-xl bg-accent-soft text-accent-text">{t.mods.runningHint}</p>}

      <Card label={plugins ? t.mods.installedPlugins : t.mods.installedMods} className="px-5 py-1.5">
        <div className="flex flex-wrap items-center gap-3 mt-3.5 mb-1">
          <h2 className="m-0 text-base font-semibold">
            {plugins ? t.mods.installedPlugins : t.mods.installedMods}
            {list && list.items.length > 0 && <span className="ml-2 text-muted font-normal">{list.items.length}</span>}
          </h2>
          <span className="flex-1" />
          <IconButton label={t.mods.openFolder} onClick={() => void api.openModsFolder(meta.id).catch(toastError)}>
            <FolderOpen size={17} aria-hidden="true" />
          </IconButton>
          {updates.length > 0 && (
            <Button size="sm" variant="primary" className="!h-10" disabled={active || busy !== null} onClick={() => void updateAll()}>
              {busy === "update-all" ? <Spinner className="!w-4 !h-4 !border-2" /> : null}
              {t.mods.updateAll(updates.length)}
            </Button>
          )}
        </div>
        <p className="mt-0 mb-2 text-[13px] text-muted">{plugins ? t.mods.pluginsHint : t.mods.modsHint}</p>
        {list && !list.online && <p className="mt-0 mb-2 text-[13px] text-warn">{t.mods.offline}</p>}

        {list === null ? (
          <div className="py-6 flex justify-center">
            <Spinner />
          </div>
        ) : list.items.length === 0 ? (
          <p className="mt-1 mb-3.5 text-muted">{plugins ? t.mods.emptyPlugins : t.mods.emptyMods}</p>
        ) : (
          <ul className="m-0 p-0 list-none flex flex-col [&>*:not(:last-child)]:border-b [&>*:not(:last-child)]:border-[#262119]">
            {list.items.map((m) => (
              <li key={m.file} className="flex flex-wrap items-center gap-3 py-3">
                <ModIcon url={m.iconUrl} title={m.title} />
                <span className="flex-[1_1_240px] min-w-0 flex flex-col gap-1">
                  {m.projectId ? (
                    <button
                      type="button"
                      onClick={() => void openExternal(modrinthUrl(m.slug, m.projectId!))}
                      className="text-left font-semibold truncate hover:text-link"
                    >
                      {m.title}
                    </button>
                  ) : (
                    <span className="font-semibold truncate">{m.title}</span>
                  )}
                  <span className="flex flex-wrap items-center gap-1.5 text-[12px] text-muted">
                    {m.version && <span className="font-mono">{m.version}</span>}
                    <span>{formatSize(m.sizeBytes)}</span>
                    {!plugins && <SideBadge side={m.clientSide} />}
                    {m.nickProtection && <Badge tone="bg-accent-soft text-accent-text">{t.mods.nickProtection}</Badge>}
                    {m.update && <Badge tone="bg-ok-bg text-ok-text">{t.mods.updateTo(m.update)}</Badge>}
                  </span>
                </span>
                <span className="flex flex-wrap items-center gap-1.5">
                  {m.update && m.projectId && (
                    <Button size="sm" variant="ghost" disabled={active || busy !== null} onClick={() => void install(m.projectId!)}>
                      {busy === m.projectId ? <Spinner className="!w-4 !h-4 !border-2" /> : null}
                      {t.mods.update}
                    </Button>
                  )}
                  <IconButton
                    label={m.nickProtection ? t.mods.nickProtectionHint : t.mods.remove}
                    className="!w-9 !h-9 hover:!text-err"
                    disabled={active || busy !== null || m.nickProtection}
                    onClick={() => setRemoveAsk(m)}
                  >
                    <Trash size={16} aria-hidden="true" />
                  </IconButton>
                </span>
              </li>
            ))}
          </ul>
        )}
      </Card>

      {meta.loader === "fabric" && list && (
        <FriendPack server={server} modsKey={list.items.map((m) => m.file).join("|")} />
      )}
      <PresetCard server={server} onApplied={() => void load()} />
      <PregenCard
        server={server}
        chunkyInstalled={list?.items.some((m) => m.projectId === CHUNKY) ?? false}
        installing={busy === CHUNKY}
        onInstallChunky={() => void install(CHUNKY)}
      />

      <Catalog
        serverId={meta.id}
        version={meta.gameVersion}
        plugins={plugins}
        installed={installedProjects}
        busy={busy}
        onInstall={(p) => void install(p)}
        modrinthUrl={modrinthUrl}
      />

      <Dialog
        open={removeAsk !== null}
        onClose={() => setRemoveAsk(null)}
        title={removeAsk ? t.mods.removeTitle(removeAsk.title) : ""}
        footer={
          <>
            <Button variant="ghost" onClick={() => setRemoveAsk(null)}>
              {t.actions.cancel}
            </Button>
            <Button variant="danger" onClick={() => removeAsk && void remove(removeAsk)}>
              <Trash size={16} aria-hidden="true" />
              {t.mods.remove}
            </Button>
          </>
        }
      >
        <p className="m-0">{t.mods.removeText}</p>
      </Dialog>
    </div>
  );
}

function Catalog({
  serverId,
  version,
  plugins,
  installed,
  busy,
  onInstall,
  modrinthUrl,
}: {
  serverId: string;
  version: string;
  plugins: boolean;
  installed: Set<string | null>;
  busy: string | null;
  onInstall: (project: string) => void;
  modrinthUrl: (slug: string | null, projectId: string) => string;
}) {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<ModHit[] | null>(null);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  const requestRef = useRef(0);

  // Поиск с небольшой задержкой, пока пользователь печатает.
  useEffect(() => {
    const request = ++requestRef.current;
    const timer = setTimeout(
      () => {
        setError(null);
        api.searchMods(serverId, query, 0).then(
          (r) => {
            if (request !== requestRef.current) return;
            setHits(r.hits);
            setTotal(r.total);
          },
          (e) => {
            if (request !== requestRef.current) return;
            setHits([]);
            setError(errorMessage(e));
          },
        );
      },
      query ? 350 : 0,
    );
    return () => clearTimeout(timer);
  }, [serverId, query]);

  async function more() {
    if (!hits) return;
    setLoadingMore(true);
    try {
      const r = await api.searchMods(serverId, query, hits.length);
      setHits([...hits, ...r.hits.filter((h) => !hits.some((x) => x.projectId === h.projectId))]);
      setTotal(r.total);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setLoadingMore(false);
    }
  }

  return (
    <Card label={t.mods.search} className="px-5 py-1.5">
      <h2 className="mt-3.5 mb-2.5 text-base font-semibold">{t.mods.search}</h2>
      <label className="relative block">
        <Search size={17} aria-hidden="true" className="absolute left-3 top-1/2 -translate-y-1/2 text-faint" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={plugins ? t.mods.searchPlaceholderPlugins : t.mods.searchPlaceholderMods}
          aria-label={t.mods.search}
          className={`${inputClass} !bg-bg !pl-9`}
        />
      </label>
      <p className="mt-2.5 mb-1 text-[13px] text-muted">{query.trim() ? t.mods.found(total) : t.mods.popular(version)}</p>

      {error ? (
        <p className="mt-1 mb-3.5 text-err">
          {t.mods.searchFailed}: {error}
        </p>
      ) : hits === null ? (
        <div className="py-6 flex justify-center">
          <Spinner />
        </div>
      ) : hits.length === 0 ? (
        <p className="mt-1 mb-3.5 text-muted">{t.mods.nothing}</p>
      ) : (
        <>
          <ul className="m-0 p-0 list-none flex flex-col [&>*:not(:last-child)]:border-b [&>*:not(:last-child)]:border-[#262119]">
            {hits.map((h) => {
              const has = installed.has(h.projectId);
              return (
                <li key={h.projectId} className="flex flex-wrap items-center gap-3 py-3">
                  <ModIcon url={h.iconUrl} title={h.title} />
                  <span className="flex-[1_1_260px] min-w-0 flex flex-col gap-1">
                    <span className="flex flex-wrap items-baseline gap-x-2">
                      <button
                        type="button"
                        onClick={() => void openExternal(modrinthUrl(h.slug, h.projectId))}
                        className="text-left font-semibold hover:text-link"
                      >
                        {h.title}
                      </button>
                      <span className="text-[12px] text-faint">{t.mods.by(h.author)}</span>
                    </span>
                    <span className="text-[13px] text-soft line-clamp-2">{h.description}</span>
                    <span className="flex flex-wrap items-center gap-1.5 text-[12px] text-muted">
                      <span>{t.mods.downloads(h.downloads)}</span>
                      {!plugins && <SideBadge side={h.clientSide} />}
                    </span>
                  </span>
                  <Button
                    size="sm"
                    variant={has ? "ghost" : "secondary"}
                    disabled={has || busy !== null}
                    onClick={() => onInstall(h.projectId)}
                    className="min-w-[110px]"
                  >
                    {busy === h.projectId ? (
                      <>
                        <Spinner className="!w-4 !h-4 !border-2" />
                        {t.mods.installing}
                      </>
                    ) : has ? (
                      t.mods.installed
                    ) : (
                      t.mods.install
                    )}
                  </Button>
                </li>
              );
            })}
          </ul>
          {hits.length < total && (
            <div className="py-3 flex justify-center">
              <Button size="sm" variant="ghost" disabled={loadingMore} onClick={() => void more()}>
                {loadingMore ? <Spinner className="!w-4 !h-4 !border-2" /> : null}
                {t.mods.more}
              </Button>
            </div>
          )}
        </>
      )}
    </Card>
  );
}

function ModIcon({ url, title }: { url: string | null; title: string }) {
  const [failed, setFailed] = useState(false);
  if (url && !failed) {
    return <img src={url} alt="" loading="lazy" onError={() => setFailed(true)} className="w-10 h-10 rounded-lg shrink-0 bg-chip object-cover" />;
  }
  return (
    <span aria-hidden="true" className="w-10 h-10 rounded-lg shrink-0 bg-chip text-soft font-bold flex items-center justify-center">
      {(title.trim()[0] ?? "?").toUpperCase()}
    </span>
  );
}

function SideBadge({ side }: { side: ClientSide }) {
  if (side === "unknown") return null;
  return <Badge tone={SIDE_TONE[side]}>{t.mods.side[side]}</Badge>;
}

function Badge({ tone, children }: { tone: string; children: string }) {
  return <span className={`px-2 py-0.5 rounded-full ${tone}`}>{children}</span>;
}
