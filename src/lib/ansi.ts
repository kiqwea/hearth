// Разбор цветов ANSI в выводе сервера (Paper красит лог) в куски с цветом.

export interface Segment {
  text: string;
  color?: string;
  bold?: boolean;
}

// Палитра подобрана под тёмный фон консоли, чтобы всё читалось.
const BASIC = ["#8d857b", "#ff8f80", "#8fe3b2", "#f2c46b", "#9fc3f2", "#e2b4e8", "#8fd8e0", "#ede6dd"];
const BRIGHT = ["#a39a8f", "#ffb0a5", "#b0f0cb", "#ffd98a", "#bdd6ff", "#f0ccf4", "#b0eef2", "#ffffff"];

// eslint-disable-next-line no-control-regex
const ESCAPE = /\u001b\[([0-9;]*)([A-Za-z])/g;

function color256(n: number): string | undefined {
  if (n < 8) return BASIC[n];
  if (n < 16) return BRIGHT[n - 8];
  if (n < 232) {
    const v = n - 16;
    const steps = [0, 95, 135, 175, 215, 255];
    return `rgb(${steps[Math.floor(v / 36)]}, ${steps[Math.floor(v / 6) % 6]}, ${steps[v % 6]})`;
  }
  const g = 8 + (n - 232) * 10;
  return `rgb(${g}, ${g}, ${g})`;
}

export function parseAnsi(input: string): Segment[] {
  const segments: Segment[] = [];
  let color: string | undefined;
  let bold = false;
  let last = 0;
  ESCAPE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = ESCAPE.exec(input)) !== null) {
    if (m.index > last) segments.push({ text: input.slice(last, m.index), color, bold });
    last = ESCAPE.lastIndex;
    if (m[2] !== "m") continue;
    const codes = m[1] === "" ? [0] : m[1].split(";").map(Number);
    for (let i = 0; i < codes.length; i++) {
      const c = codes[i];
      if (c === 0) {
        color = undefined;
        bold = false;
      } else if (c === 1) bold = true;
      else if (c === 22) bold = false;
      else if (c === 39) color = undefined;
      else if (c >= 30 && c <= 37) color = BASIC[c - 30];
      else if (c >= 90 && c <= 97) color = BRIGHT[c - 90];
      else if (c === 38 && codes[i + 1] === 5) {
        color = color256(codes[i + 2]);
        i += 2;
      } else if (c === 38 && codes[i + 1] === 2) {
        color = `rgb(${codes[i + 2]}, ${codes[i + 3]}, ${codes[i + 4]})`;
        i += 4;
      }
    }
  }
  if (last < input.length) segments.push({ text: input.slice(last), color, bold });
  return segments.filter((s) => s.text.length > 0);
}

export function stripAnsi(input: string): string {
  return input.replace(ESCAPE, "");
}

export type LogLevel = "info" | "warn" | "error";

export function logLevel(text: string): LogLevel {
  const plain = stripAnsi(text);
  if (/\b(ERROR|SEVERE|FATAL)\b|Exception\b/.test(plain)) return "error";
  if (/\bWARN(ING)?\b/.test(plain)) return "warn";
  return "info";
}
