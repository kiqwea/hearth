// График величины во времени в стиле Hearth: тонкая линия цвета акцента,
// мягкая заливка, пунктирная линия-ориентир, перекрестие с подсказкой.
// Один ряд — одна ось: разные величины рисуются отдельными графиками.

import { useEffect, useId, useMemo, useRef, useState, type PointerEvent } from "react";

export interface ChartPoint {
  at: number;
  value: number | null;
}

/** Если между замерами больше этого, сервер был выключен: линию рвём. */
const GAP_MS = 20_000;
const PAD = { left: 46, right: 14, top: 12, bottom: 24 };

export function TimeChart({
  points,
  from,
  to,
  yMax,
  ticks,
  format,
  label,
  height = 160,
  reference,
  step = false,
  tooltip,
  timeLabels,
}: {
  points: ChartPoint[];
  from: number;
  to: number;
  yMax: number;
  /** Значения горизонтальных линий сетки (с подписями слева). */
  ticks: number[];
  format: (v: number) => string;
  /** Описание для экранного диктора. */
  label: string;
  height?: number;
  /** Ориентир: «выделено 6 ГБ», «норма 20 TPS». */
  reference?: { value: number; label: string };
  /** Ступеньки (для целых величин вроде числа игроков). */
  step?: boolean;
  /** Дополнительные строки подсказки для точки. */
  tooltip?: (p: ChartPoint) => string[];
  /** Подписи под осью: слева, по центру, справа. */
  timeLabels: [string, string, string];
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(600);
  const [hover, setHover] = useState<ChartPoint | null>(null);
  const gradient = useId();

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(([entry]) => setWidth(Math.max(200, entry.contentRect.width)));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const plotW = width - PAD.left - PAD.right;
  const plotH = height - PAD.top - PAD.bottom;
  const x = (at: number) => PAD.left + ((at - from) / Math.max(1, to - from)) * plotW;
  const y = (v: number) => PAD.top + plotH - (Math.min(v, yMax) / yMax) * plotH;
  const baseline = PAD.top + plotH;

  const visible = useMemo(() => points.filter((p) => p.at >= from - GAP_MS && p.at <= to), [points, from, to]);

  // Отрезки без разрывов: пропуск значения или долгая пауза между замерами.
  const segments = useMemo(() => {
    const out: ChartPoint[][] = [];
    let cur: ChartPoint[] = [];
    let prevAt = -Infinity;
    for (const p of visible) {
      if (p.value === null || p.at - prevAt > GAP_MS) {
        if (cur.length) out.push(cur);
        cur = [];
      }
      if (p.value !== null) cur.push(p);
      prevAt = p.at;
    }
    if (cur.length) out.push(cur);
    return out;
  }, [visible]);

  const linePath = (seg: ChartPoint[]) =>
    seg
      .map((p, i) => {
        const px = Math.max(PAD.left, x(p.at));
        const py = y(p.value!);
        if (i === 0) return `M${px},${py}`;
        return step ? `H${px}V${py}` : `L${px},${py}`;
      })
      .join("");
  const areaPath = (seg: ChartPoint[]) =>
    `${linePath(seg)}V${baseline}H${Math.max(PAD.left, x(seg[0].at))}Z`;

  const lastSeg = segments[segments.length - 1];
  const last = lastSeg ? lastSeg[lastSeg.length - 1] : null;

  function onMove(e: PointerEvent<SVGSVGElement>) {
    const rect = e.currentTarget.getBoundingClientRect();
    const px = e.clientX - rect.left;
    let best: ChartPoint | null = null;
    let bestDist = Infinity;
    for (const p of visible) {
      if (p.value === null) continue;
      const d = Math.abs(x(p.at) - px);
      if (d < bestDist) {
        best = p;
        bestDist = d;
      }
    }
    setHover(best && bestDist < 40 ? best : null);
  }

  const time = (at: number) => new Date(at).toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  const tipLeft = hover ? Math.min(Math.max(x(hover.at), 70), width - 70) : 0;

  return (
    <div ref={ref} className="relative w-full select-none">
      <svg
        width={width}
        height={height}
        role="img"
        aria-label={label}
        onPointerMove={onMove}
        onPointerLeave={() => setHover(null)}
        className="block overflow-visible"
      >
        <defs>
          <linearGradient id={gradient} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--color-accent)" stopOpacity="0.32" />
            <stop offset="100%" stopColor="var(--color-accent)" stopOpacity="0" />
          </linearGradient>
        </defs>

        {ticks.map((v) => (
          <g key={v}>
            <line x1={PAD.left} x2={width - PAD.right} y1={y(v)} y2={y(v)} stroke="var(--color-line)" strokeWidth="1" />
            <text x={PAD.left - 8} y={y(v)} dy="0.32em" textAnchor="end" fontSize="11" fill="var(--color-faint)">
              {format(v)}
            </text>
          </g>
        ))}

        {reference && (
          <g>
            <line
              x1={PAD.left}
              x2={width - PAD.right}
              y1={y(reference.value)}
              y2={y(reference.value)}
              stroke="var(--color-soft)"
              strokeWidth="1.5"
              strokeDasharray="5 5"
              opacity="0.7"
            />
            <text x={width - PAD.right} y={y(reference.value) - 6} textAnchor="end" fontSize="11" fill="var(--color-muted)">
              {reference.label}
            </text>
          </g>
        )}

        {segments.map((seg, i) => (
          <g key={i}>
            <path d={areaPath(seg)} fill={`url(#${gradient})`} />
            <path d={linePath(seg)} fill="none" stroke="var(--color-accent)" strokeWidth="2" strokeLinejoin="round" strokeLinecap="round" />
          </g>
        ))}

        {last && !hover && (
          <circle cx={x(last.at)} cy={y(last.value!)} r="4" fill="var(--color-accent)" stroke="var(--color-card)" strokeWidth="2" />
        )}

        {hover && (
          <g pointerEvents="none">
            <line x1={x(hover.at)} x2={x(hover.at)} y1={PAD.top} y2={baseline} stroke="var(--color-line-strong)" strokeWidth="1" />
            <circle cx={x(hover.at)} cy={y(hover.value!)} r="5" fill="var(--color-accent)" stroke="var(--color-card)" strokeWidth="2" />
          </g>
        )}

        {timeLabels.map((text, i) => (
          <text
            key={i}
            x={i === 0 ? PAD.left : i === 1 ? PAD.left + plotW / 2 : width - PAD.right}
            y={height - 6}
            textAnchor={i === 0 ? "start" : i === 1 ? "middle" : "end"}
            fontSize="11"
            fill="var(--color-faint)"
          >
            {text}
          </text>
        ))}
      </svg>

      {hover && (
        <div
          className="pointer-events-none absolute top-0 -translate-x-1/2 -translate-y-[calc(100%+6px)] px-2.5 py-1.5 rounded-lg bg-raised border border-line-strong shadow-lg text-[12px] whitespace-nowrap"
          style={{ left: tipLeft }}
        >
          <div className="font-semibold text-ink">{format(hover.value!)}</div>
          {tooltip?.(hover).map((line) => (
            <div key={line} className="text-soft">
              {line}
            </div>
          ))}
          <div className="text-faint">{time(hover.at)}</div>
        </div>
      )}
    </div>
  );
}
