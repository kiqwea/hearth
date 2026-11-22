// «Для друзей через интернет». Кнопки «включить» нет: при запуске сервера Hearth
// сам пробует прямое подключение (порт на роутере), иначе playit.gg. Вмешательство
// нужно, только если playit ещё не подключён: войти в аккаунт может лишь пользователь.

import { Copy, ExternalLink, Globe } from "lucide-react";

import type { ServerSummary } from "../../bindings/ServerSummary";
import { Button, IconButton, Spinner, StatusDot } from "../../components/ui";
import { t } from "../../i18n";
import { api, openExternal } from "../../lib/api";
import { copyText, isActive, type Tone } from "../../lib/format";
import { useHearth } from "../../lib/store";

export function FriendAccess({ server }: { server: ServerSummary }) {
  const { meta, status } = server;
  const playit = useHearth((s) => s.playit);
  const busy = useHearth((s) => !!s.accessBusy[meta.id]);
  const switchToPlayit = useHearth((s) => s.switchToPlayit);
  const connect = useHearth((s) => s.connectFriendAccess);
  const enable = useHearth((s) => s.enableFriendAccess);
  const cancelClaim = useHearth((s) => s.cancelPlayitClaim);
  const toast = useHearth((s) => s.toast);
  const toastError = useHearth((s) => s.toastError);

  const tunnel = meta.playit;
  const info = tunnel ? playit?.tunnels.find((x) => x.id === tunnel.tunnelId) : undefined;
  const linked = playit?.linked ?? false;
  const claimUrl = playit?.claimUrl ?? null;

  const openDashboard = async () => {
    try {
      await openExternal(await api.playitDashboardUrl());
    } catch (e) {
      toastError(e);
    }
  };

  let body;
  if (meta.directAccess) {
    const direct = server.direct;
    const address = direct?.address ?? null;
    const state = direct?.state ?? "off";
    let tone: Tone = "off";
    let line: string = address ? t.access.directStopped : t.access.directUnknown;
    if (state === "opening") {
      tone = "busy";
      line = t.access.directOpening;
    } else if (state === "open") {
      tone = "ok";
      line = t.access.directOpen;
    } else if (state === "failed") {
      tone = "err";
      line = t.access.directError(direct?.error ?? t.errors.generic);
    }
    body = (
      <div className="flex flex-col gap-2.5">
        <AddressRow address={address} placeholder={t.access.directNoAddress} />
        <span className="flex flex-wrap items-center gap-2 text-[13px] text-soft" role="status">
          <StatusDot tone={tone} />
          <span className={tone === "err" ? "text-err" : undefined}>{line}</span>
          {state === "failed" && (
            <Button size="sm" variant="ghost" disabled={busy} onClick={() => void switchToPlayit(meta.id)}>
              {t.access.usePlayit}
            </Button>
          )}
        </span>
        {address && (
          <span className="text-[13px] text-muted">
            {t.access.howTo} {t.access.selfHint}{" "}
            <span className="font-mono selectable text-soft">{meta.port === 25565 ? "localhost" : `localhost:${meta.port}`}</span>
          </span>
        )}
        {state !== "failed" && (
          <span className="text-[13px] text-muted">
            {t.access.directHint}{" "}
            <button type="button" className="underline text-link" disabled={busy} onClick={() => void switchToPlayit(meta.id)}>
              {t.access.usePlayit}
            </button>
          </span>
        )}
      </div>
    );
  } else if (claimUrl) {
    body = (
      <div className="flex flex-col gap-3">
        <div className="flex items-start gap-3">
          <Spinner className="!w-5 !h-5 !border-2 mt-0.5" />
          <span className="flex flex-col gap-1">
            <span className="font-semibold text-ink">{t.access.claimTitle}</span>
            <span className="text-soft text-[13px] leading-relaxed">{t.access.claimText}</span>
          </span>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button size="sm" variant="secondary" onClick={() => void openExternal(claimUrl)}>
            <ExternalLink size={14} aria-hidden="true" />
            {t.access.claimOpen}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={async () => {
              if (await copyText(claimUrl)) toast("success", `${t.actions.copied}: ${claimUrl}`);
            }}
          >
            <Copy size={14} aria-hidden="true" />
            {t.access.claimLink}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => void cancelClaim()}>
            {t.actions.cancel}
          </Button>
        </div>
      </div>
    );
  } else if (tunnel && !linked) {
    // Адрес playit был, но Hearth отключили от playit.
    body = (
      <div className="flex flex-col gap-3">
        <span className="text-soft leading-relaxed">{t.playitSettings.notLinked}</span>
        <PlayitLogin onLogin={() => void connect(meta.id, { skipDirect: true })} error={playit?.claimError ?? null} />
      </div>
    );
  } else if (!tunnel) {
    // Доступ ещё не настроен: Hearth делает это сам при запуске сервера.
    const direct = server.direct;
    const failed = direct?.state === "failed";
    if (busy || (failed && linked && isActive(status))) {
      body = <Progress text={t.access.enabling} />;
    } else if (!isActive(status)) {
      body = <span className="text-soft leading-relaxed">{t.access.autoStopped}</span>;
    } else if (failed) {
      body = (
        <div className="flex flex-col gap-3">
          <span className="text-[13px] text-muted">{t.access.directFailed(direct?.error ?? t.errors.generic)}</span>
          <span className="text-soft leading-relaxed">{t.access.needPlayit}</span>
          <PlayitLogin onLogin={() => void connect(meta.id, { skipDirect: true })} error={playit?.claimError ?? null} />
        </div>
      );
    } else {
      body = <Progress text={t.access.autoSetting} />;
    }
  } else {
    const missing = playit?.synced && !info;
    const address = info?.address ?? null;
    const running = status.state === "running";
    const agent = playit?.agent ?? "off";

    let tone: Tone = "off";
    let line: string = t.access.stateStopped;
    if (missing) {
      tone = "warn";
      line = t.access.missing;
    } else if (info?.disabled) {
      tone = "warn";
      line = t.access.disabled(info.disabled);
    } else if (agent === "error") {
      tone = "err";
      line = playit?.agentError ?? t.errors.generic;
    } else if (running && agent === "online") {
      tone = address ? "ok" : "busy";
      line = address ? t.access.stateOnline : (info?.pending ?? t.access.addressPending);
    } else if (agent === "downloading" && isActive(status)) {
      tone = "busy";
      line = t.access.stateDownloading;
    } else if (agent === "starting" && isActive(status)) {
      tone = "busy";
      line = t.access.stateStarting;
    } else if (isActive(status)) {
      tone = "busy";
      line = t.access.stateWaiting;
    }

    body = (
      <div className="flex flex-col gap-2.5">
        <AddressRow address={address} />
        <span className="flex items-center gap-2 text-[13px] text-soft" role="status">
          <StatusDot tone={tone} />
          <span className={tone === "err" ? "text-err" : undefined}>{line}</span>
          {agent === "error" && (
            <Button size="sm" variant="ghost" className="ml-1" onClick={() => api.playitRetry().catch(toastError)}>
              {t.access.retry}
            </Button>
          )}
          {missing && (
            <Button size="sm" variant="ghost" className="ml-1" disabled={busy} onClick={() => void enable(meta.id)}>
              {t.access.recreate}
            </Button>
          )}
        </span>
        {address && (
          <span className="text-[13px] text-muted">
            {t.access.howTo} {t.access.selfHint}{" "}
            <span className="font-mono selectable text-soft">{meta.port === 25565 ? "localhost" : `localhost:${meta.port}`}</span>
          </span>
        )}
        {playit?.notices
          .filter((n) => n.important)
          .slice(0, 1)
          .map((n) => (
            <span key={n.message} className="text-[13px] text-warn">
              {n.message}{" "}
              {n.link && (
                <button type="button" className="underline" onClick={() => void openExternal(n.link!)}>
                  {t.access.dashboard}
                </button>
              )}
            </span>
          ))}
        {playit?.guest && (
          <span className="text-[13px] text-muted">
            {t.access.guest}{" "}
            <button type="button" className="underline text-link" onClick={() => void openDashboard()}>
              {t.access.dashboard}
            </button>
          </span>
        )}
      </div>
    );
  }

  return (
    <div className="flex items-start gap-3 px-3.5 py-3 rounded-[10px] bg-raised">
      <Globe size={20} className="text-faint shrink-0 mt-0.5" aria-hidden="true" />
      <div className="flex-1 min-w-0 flex flex-col gap-2">
        <span className="text-xs text-muted">{t.access.title}</span>
        {body}
      </div>
    </div>
  );
}

function Progress({ text }: { text: string }) {
  return (
    <span className="flex items-center gap-2.5 text-soft">
      <Spinner className="!w-4 !h-4 !border-2" />
      {text}
    </span>
  );
}

/** Единственная кнопка, без которой не обойтись: войти в playit может только сам пользователь. */
function PlayitLogin({ onLogin, error }: { onLogin: () => void; error: string | null }) {
  return (
    <>
      {error && (
        <span role="alert" className="text-err text-[13px]">
          {error}
        </span>
      )}
      <div className="flex flex-wrap items-center gap-3">
        <Button variant="primary" size="sm" onClick={onLogin}>
          <ExternalLink size={14} aria-hidden="true" />
          {t.access.loginPlayit}
        </Button>
        <span className="text-[13px] text-muted">{t.access.loginOnce}</span>
      </div>
    </>
  );
}

/** Адрес крупно и кнопка «Копировать». */
function AddressRow({ address, placeholder = t.access.addressPending }: { address: string | null; placeholder?: string }) {
  const toast = useHearth((s) => s.toast);
  return (
    <div className="flex items-center gap-2.5 flex-wrap">
      <span
        className={`flex-[1_1_220px] min-w-0 font-mono text-[17px] font-semibold selectable truncate ${address ? "text-ink" : "text-faint"}`}
      >
        {address ?? placeholder}
      </span>
      {address && (
        <IconButton
          label={t.access.copyAddress}
          onClick={async () => {
            if (await copyText(address)) toast("success", `${t.actions.copied}: ${address}`);
          }}
        >
          <Copy size={16} aria-hidden="true" />
        </IconButton>
      )}
    </div>
  );
}
