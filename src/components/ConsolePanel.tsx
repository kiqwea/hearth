import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Copy } from "lucide-react";

import type { ConsoleLine } from "../bindings/ConsoleLine";
import { t } from "../i18n";
import { logLevel, parseAnsi, stripAnsi, type LogLevel } from "../lib/ansi";
import { api } from "../lib/api";
import { copyText, isActive } from "../lib/format";
import { useHearth } from "../lib/store";
import { Button, IconButton } from "./ui";

const QUICK_COMMANDS = ["time set day", "weather clear", "list", "save-all"];
const EMPTY: ConsoleLine[] = [];

// История команд живёт, пока открыто приложение, отдельно для каждого сервера.
const history = new Map<string, string[]>();

type Filter = "all" | "warn" | "error";

export function ConsolePanel({ id, compact = false, onOpenFull }: { id: string; compact?: boolean; onOpenFull?: () => void }) {
  const lines = useHearth((s) => s.consoles[id] ?? EMPTY);
  const status = useHearth((s) => s.servers[id]?.status);
  const loadConsole = useHearth((s) => s.loadConsole);
  const toast = useHearth((s) => s.toast);
  const toastError = useHearth((s) => s.toastError);

  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [autoscroll, setAutoscroll] = useState(true);
  const [command, setCommand] = useState("");
  const [historyIndex, setHistoryIndex] = useState<number | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const stuckToBottom = useRef(true);

  useEffect(() => {
    void loadConsole(id);
  }, [id, loadConsole]);

  const levels = useMemo(() => lines.map((l) => (l.stream === "stdout" || l.stream === "stderr" ? logLevel(l.text) : "info")), [lines]);
  const counts = useMemo(
    () => ({ warn: levels.filter((l) => l === "warn").length, error: levels.filter((l) => l === "error").length }),
    [levels],
  );

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    const items = lines.map((line, i) => ({ line, level: levels[i] }));
    const filtered = items.filter(({ line, level }) => {
      if (filter === "warn" && level !== "warn") return false;
      if (filter === "error" && level !== "error") return false;
      if (q && !stripAnsi(line.text).toLowerCase().includes(q)) return false;
      return true;
    });
    return compact ? filtered.slice(-8) : filtered;
  }, [lines, levels, query, filter, compact]);

  useEffect(() => {
    const el = scrollRef.current;
    if (el && (compact || (autoscroll && stuckToBottom.current))) el.scrollTop = el.scrollHeight;
  }, [visible, autoscroll, compact]);

  const running = status ? isActive(status) : false;

  async function send(text: string) {
    const value = text.trim();
    if (!value) return;
    const list = history.get(id) ?? [];
    if (list[list.length - 1] !== value) list.push(value);
    history.set(id, list.slice(-100));
    setHistoryIndex(null);
    setCommand("");
    stuckToBottom.current = true;
    try {
      await api.sendCommand(id, value);
    } catch (e) {
      toastError(e);
    }
  }

  function onKey(e: KeyboardEvent<HTMLInputElement>) {
    const list = history.get(id) ?? [];
    if (e.key === "Enter") {
      e.preventDefault();
      void send(command);
    } else if (e.key === "ArrowUp" && list.length) {
      e.preventDefault();
      const next = historyIndex === null ? list.length - 1 : Math.max(0, historyIndex - 1);
      setHistoryIndex(next);
      setCommand(list[next]);
    } else if (e.key === "ArrowDown" && historyIndex !== null) {
      e.preventDefault();
      const next = historyIndex + 1;
      if (next >= list.length) {
        setHistoryIndex(null);
        setCommand("");
      } else {
        setHistoryIndex(next);
        setCommand(list[next]);
      }
    }
  }

  async function copyAll() {
    const ok = await copyText(lines.map((l) => stripAnsi(l.text)).join("\n"));
    if (ok) toast("success", t.actions.copied);
  }

  return (
    <section
      aria-label={t.overview.console}
      className={`bg-term border border-line rounded-[14px] overflow-hidden flex flex-col ${compact ? "" : "flex-1 min-h-0"}`}
    >
      {compact ? (
        <div className="flex items-center justify-between px-[18px] py-3 border-b border-line-soft">
          <h2 className="m-0 text-[15px] font-semibold">{t.overview.console}</h2>
          {onOpenFull && (
            <button type="button" onClick={onOpenFull} className="text-[13px] text-link hover:text-accent-text">
              {t.overview.openConsole}
            </button>
          )}
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-2.5 px-3 py-2.5 border-b border-line-soft bg-[#161310]">
          <label htmlFor={`search-${id}`} className="sr-only">
            {t.console.search}
          </label>
          <input
            id={`search-${id}`}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t.console.search}
            className="flex-[1_1_200px] h-9 px-3 rounded-lg border border-line bg-card text-ink text-[13px] placeholder:text-faint"
          />
          <div role="group" aria-label="Фильтр строк" className="flex gap-1">
            <FilterButton active={filter === "all"} onClick={() => setFilter("all")} className="text-ink">
              {t.console.all}
            </FilterButton>
            <FilterButton active={filter === "warn"} onClick={() => setFilter("warn")} className="text-warn">
              {t.console.warnings} · {counts.warn}
            </FilterButton>
            <FilterButton active={filter === "error"} onClick={() => setFilter("error")} className="text-err">
              {t.console.errors} · {counts.error}
            </FilterButton>
          </div>
          <label className="flex items-center gap-2 text-[13px] text-soft">
            <input type="checkbox" checked={autoscroll} onChange={(e) => setAutoscroll(e.target.checked)} className="w-4 h-4" />
            {t.console.autoscroll}
          </label>
          <IconButton label={t.console.copyLog} onClick={copyAll} className="!w-9 !h-9 !bg-transparent">
            <Copy size={16} aria-hidden="true" />
          </IconButton>
        </div>
      )}

      <div
        ref={scrollRef}
        onScroll={(e) => {
          const el = e.currentTarget;
          stuckToBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        }}
        className={`selectable overflow-y-auto px-[18px] py-3.5 font-mono text-[12.5px] leading-[1.7] text-soft whitespace-pre-wrap break-words ${
          compact ? "h-[208px]" : "flex-1 min-h-[300px]"
        }`}
      >
        {visible.length === 0 ? (
          <span className="text-faint">{t.console.empty}</span>
        ) : (
          visible.map(({ line, level }) => <Line key={line.seq} line={line} level={level} />)
        )}
      </div>

      <div className={`flex flex-col gap-2.5 p-3 border-t border-line-soft ${compact ? "" : "bg-[#161310]"}`}>
        {!compact && running && (
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-xs text-muted mr-1">{t.console.quick}</span>
            {QUICK_COMMANDS.map((c) => (
              <button
                key={c}
                type="button"
                onClick={() => void send(c)}
                className="h-8 px-2.5 rounded-lg border border-line text-soft font-mono text-xs hover:bg-hover"
              >
                {c}
              </button>
            ))}
          </div>
        )}
        {running ? (
          <div className="flex gap-2">
            <span aria-hidden="true" className="flex items-center px-1.5 text-accent font-mono font-semibold">
              &gt;
            </span>
            <label htmlFor={`cmd-${id}-${compact ? "c" : "f"}`} className="sr-only">
              {t.console.command}
            </label>
            <input
              id={`cmd-${id}-${compact ? "c" : "f"}`}
              value={command}
              onChange={(e) => {
                setCommand(e.target.value);
                setHistoryIndex(null);
              }}
              onKeyDown={onKey}
              placeholder={compact ? t.console.placeholderShort : t.console.placeholder}
              autoComplete="off"
              spellCheck={false}
              className="flex-1 min-w-0 h-10 px-3 rounded-[9px] border border-line bg-side text-ink font-mono text-[13px] placeholder:text-faint"
            />
            <Button variant={compact ? "secondary" : "primary"} onClick={() => void send(command)} className="!h-10">
              {t.actions.send}
            </Button>
          </div>
        ) : (
          <span className="text-[13px] text-muted px-1">{t.console.notRunning}</span>
        )}
      </div>
    </section>
  );
}

function FilterButton({
  active,
  onClick,
  className,
  children,
}: {
  active: boolean;
  onClick: () => void;
  className: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={`h-9 px-3 rounded-lg text-[13px] ${active ? "bg-hover font-semibold" : "hover:bg-[#211c18]"} ${className}`}
    >
      {children}
    </button>
  );
}

const LEVEL_COLOR: Record<LogLevel, string | undefined> = {
  info: undefined,
  warn: "var(--color-warn)",
  error: "var(--color-err)",
};

function Line({ line, level }: { line: ConsoleLine; level: LogLevel }) {
  if (line.stream === "input") {
    return <div className="text-accent-text">&gt; {line.text}</div>;
  }
  if (line.stream === "system") {
    return (
      <div>
        <span className="text-faint">Hearth · </span>
        <span className="text-link">{line.text}</span>
      </div>
    );
  }
  const segments = parseAnsi(line.text);
  const fallback = LEVEL_COLOR[level];
  return (
    <div>
      {segments.map((s, i) => (
        <span key={i} style={{ color: s.color ?? fallback, fontWeight: s.bold ? 600 : undefined }}>
          {s.text}
        </span>
      ))}
    </div>
  );
}
