// Вкладка «Игроки»: кто онлайн, кто заходил, операторы, белый список, баны.
// Действия выполняет сам сервер (через служебный канал Hearth), поэтому они
// доступны, только пока сервер работает. Списки читаются из его файлов всегда.

import { useCallback, useEffect, useState } from "react";

import type { PlayerAction } from "../../bindings/PlayerAction";
import type { PlayerInfo } from "../../bindings/PlayerInfo";
import type { ServerSummary } from "../../bindings/ServerSummary";
import { Button, Card, Dialog, inputClass, StatusDot } from "../../components/ui";
import { t } from "../../i18n";
import { api } from "../../lib/api";
import { avatarColors, initial } from "../../lib/format";
import { useHearth } from "../../lib/store";
import { Monitoring } from "./Monitoring";

const NICK = /^[A-Za-z0-9_]{1,16}$/;

type Ask = { action: "kick" | "ban"; name: string };

export function PlayersTab({ server }: { server: ServerSummary }) {
  const { meta, status } = server;
  const toast = useHearth((s) => s.toast);
  const toastError = useHearth((s) => s.toastError);
  const [players, setPlayers] = useState<PlayerInfo[] | null>(null);
  const [whitelistOn, setWhitelistOn] = useState(false);
  const [nick, setNick] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [ask, setAsk] = useState<Ask | null>(null);
  const [reason, setReason] = useState("");

  const running = status.state === "running";
  const onlineKey = server.stats?.online.join(",") ?? "";

  const load = useCallback(async () => {
    try {
      const [list, props] = await Promise.all([api.getPlayers(meta.id), api.getProperties(meta.id)]);
      setPlayers(list);
      setWhitelistOn(props.find((p) => p.key === "white-list")?.value === "true");
    } catch (e) {
      toastError(e);
    }
  }, [meta.id, toastError]);

  // Перечитываем, когда кто-то зашёл или вышел.
  useEffect(() => {
    void load();
  }, [load, onlineKey, status.state]);

  async function act(action: PlayerAction, name: string, why: string | null = null) {
    setBusy(`${action}:${name}`);
    try {
      const reply = await api.playerAction(meta.id, action, name, why);
      toast("success", reply || t.players.done);
      // Сервер записывает списки в файлы не мгновенно.
      setTimeout(() => void load(), 600);
    } catch (e) {
      toastError(e);
    } finally {
      setBusy(null);
    }
  }

  const nickOk = NICK.test(nick.trim());

  return (
    <div className="flex flex-col gap-[18px] max-w-[860px]">
      <Monitoring server={server} />

      {!running && (
        <p className="m-0 px-4 py-3 rounded-xl bg-accent-soft text-accent-text">{t.players.stopped}</p>
      )}

      <Card label={t.players.add} className="px-5 py-4 flex flex-col gap-3">
        <h2 className="m-0 text-base font-semibold">{t.players.add}</h2>
        <div className="flex flex-wrap gap-2.5">
          <input
            value={nick}
            onChange={(e) => setNick(e.target.value)}
            placeholder={t.players.nickPlaceholder}
            aria-label={t.players.nickPlaceholder}
            maxLength={16}
            className={`${inputClass} !h-10 flex-[1_1_220px] font-mono ${nick && !nickOk ? "border-err" : ""}`}
          />
          <Button
            size="sm"
            className="!h-10"
            disabled={!running || !nickOk || busy !== null}
            onClick={() => void act("whitelistAdd", nick.trim()).then(() => setNick(""))}
          >
            {t.players.whitelistAdd}
          </Button>
          <Button
            size="sm"
            className="!h-10"
            disabled={!running || !nickOk || busy !== null}
            onClick={() => void act("op", nick.trim()).then(() => setNick(""))}
          >
            {t.players.op}
          </Button>
        </div>
        <span className="text-[13px] text-muted">{whitelistOn ? t.players.whitelistOn : t.players.whitelistOff}</span>
      </Card>

      <Card label={t.players.title} className="px-5 py-1.5">
        <h2 className="mt-3.5 mb-1.5 text-base font-semibold">
          {t.players.title}
          {players && players.length > 0 && <span className="ml-2 text-muted font-normal">{players.length}</span>}
        </h2>
        {players === null ? null : players.length === 0 ? (
          <p className="mt-0 mb-3.5 text-muted">{t.players.empty}</p>
        ) : (
          <ul className="m-0 p-0 list-none flex flex-col [&>*:not(:last-child)]:border-b [&>*:not(:last-child)]:border-[#262119]">
            {players.map((p) => (
              <PlayerRow
                key={p.name}
                player={p}
                running={running}
                busy={busy}
                onAction={(action) => {
                  if (action === "kick" || action === "ban") {
                    setReason("");
                    setAsk({ action, name: p.name });
                  } else {
                    void act(action, p.name);
                  }
                }}
              />
            ))}
          </ul>
        )}
      </Card>

      <Dialog
        open={ask !== null}
        onClose={() => setAsk(null)}
        title={ask ? (ask.action === "kick" ? t.players.kickTitle(ask.name) : t.players.banTitle(ask.name)) : ""}
        footer={
          <>
            <Button variant="ghost" onClick={() => setAsk(null)}>
              {t.actions.cancel}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                if (ask) void act(ask.action, ask.name, reason.trim() || null);
                setAsk(null);
              }}
            >
              {ask?.action === "kick" ? t.players.kick : t.players.ban}
            </Button>
          </>
        }
      >
        <p className="m-0">{ask?.action === "kick" ? t.players.kickText : t.players.banText}</p>
        <input
          value={reason}
          onChange={(e) => setReason(e.target.value)}
          placeholder={t.players.reasonPlaceholder}
          aria-label={t.players.reasonPlaceholder}
          maxLength={100}
          className={`${inputClass} !bg-bg`}
        />
      </Dialog>
    </div>
  );
}

function PlayerRow({
  player: p,
  running,
  busy,
  onAction,
}: {
  player: PlayerInfo;
  running: boolean;
  busy: string | null;
  onAction: (action: PlayerAction) => void;
}) {
  const colors = avatarColors(p.name.toLowerCase());
  const disabled = !running || busy !== null;
  const button = (action: PlayerAction, label: string, variant: "ghost" | "danger" = "ghost") => (
    <Button size="sm" variant={variant} disabled={disabled} onClick={() => onAction(action)}>
      {label}
    </Button>
  );
  return (
    <li className="flex flex-wrap items-center gap-3 py-3">
      <span
        aria-hidden="true"
        className="w-9 h-9 rounded-lg flex items-center justify-center font-bold shrink-0"
        style={{ background: colors.bg, color: colors.fg }}
      >
        {initial(p.name)}
      </span>
      <span className="flex-[1_1_200px] min-w-0 flex flex-col gap-1">
        <span className="font-semibold font-mono truncate">{p.name}</span>
        <span className="flex flex-wrap items-center gap-1.5 text-[12px]">
          {p.online && (
            <span className="flex items-center gap-1.5 text-ok-text">
              <StatusDot tone="ok" className="!w-[7px] !h-[7px]" />
              {t.players.online}
            </span>
          )}
          {p.op && <Badge>{t.players.isOp}</Badge>}
          {p.whitelisted && <Badge>{t.players.isWhitelisted}</Badge>}
          {p.banned && (
            <span className="px-2 py-0.5 rounded-full bg-err-bg text-err">
              {p.banReason ? t.players.bannedFor(p.banReason) : t.players.isBanned}
            </span>
          )}
        </span>
      </span>
      <span className="flex flex-wrap gap-1.5">
        {p.online && button("kick", t.players.kick)}
        {p.op ? button("deop", t.players.deop) : button("op", t.players.op)}
        {p.whitelisted ? button("whitelistRemove", t.players.whitelistRemove) : button("whitelistAdd", t.players.whitelistAdd)}
        {p.banned ? button("pardon", t.players.pardon) : button("ban", t.players.ban, "danger")}
      </span>
    </li>
  );
}

function Badge({ children }: { children: string }) {
  return <span className="px-2 py-0.5 rounded-full bg-chip text-soft">{children}</span>;
}
