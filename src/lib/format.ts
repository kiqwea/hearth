import type { Loader } from "../bindings/Loader";
import type { ServerMeta } from "../bindings/ServerMeta";
import type { ServerStatus } from "../bindings/ServerStatus";
import { t } from "../i18n";

export type Tone = "ok" | "warn" | "err" | "off" | "busy";

export function loaderName(loader: Loader): string {
  return t.loader[loader].name;
}

export function serverTitle(meta: ServerMeta): string {
  return `${loaderName(meta.loader)} ${meta.gameVersion}`;
}

export function statusText(status: ServerStatus): string {
  if (status.state === "crashed") return t.status.crashedCode(status.code);
  return t.status[status.state];
}

export function statusTone(status: ServerStatus): Tone {
  switch (status.state) {
    case "running":
      return "ok";
    case "crashed":
      return "err";
    case "stopped":
      return "off";
    default:
      return "busy";
  }
}

export function isActive(status: ServerStatus): boolean {
  return status.state === "starting" || status.state === "running" || status.state === "stopping";
}

export function gb(mb: number): number {
  return Math.round((mb / 1024) * 10) / 10;
}

export function formatGb(mb: number): string {
  return `${String(gb(mb)).replace(".", ",")} ГБ`;
}

export function formatMb(bytes: number): string {
  return `${Math.round(bytes / 1_000_000)} МБ`;
}

/** Размер файла: «850 МБ», «1,4 ГБ». */
export function formatSize(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  if (mb < 1) return bytes === 0 ? "0 МБ" : "меньше 1 МБ";
  if (mb < 1024) return `${Math.round(mb)} МБ`;
  return `${String(Math.round((mb / 1024) * 10) / 10).replace(".", ",")} ГБ`;
}

/** «сегодня в 18:05», «вчера в 09:30», «20 ноября в 14:00». */
export function formatWhen(iso: string): string {
  const d = new Date(iso);
  const time = d.toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" });
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(new Date()) - day(d)) / 86_400_000);
  if (diff === 0) return `сегодня в ${time}`;
  if (diff === 1) return `вчера в ${time}`;
  const sameYear = d.getFullYear() === new Date().getFullYear();
  const date = d.toLocaleDateString("ru-RU", { day: "numeric", month: "long", ...(sameYear ? {} : { year: "numeric" }) });
  return `${date} в ${time}`;
}

export function formatUptime(ms: number): string {
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 1) return "меньше минуты";
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return h > 0 ? `${h} ч ${m} мин` : `${m} мин`;
}

/** Адрес для подключения: стандартный порт Minecraft можно не писать. */
export function withPort(host: string, port: number): string {
  return port === 25565 ? host : `${host}:${port}`;
}

/** Первая буква названия для значка сервера. */
export function initial(name: string): string {
  return (name.trim()[0] ?? "?").toUpperCase();
}

const AVATAR_COLORS: Array<[string, string]> = [
  ["#3a2a1c", "#f3b46b"],
  ["#23272a", "#9fc3d6"],
  ["#262a1f", "#b9d68c"],
  ["#2f2230", "#e2b4e8"],
  ["#2c2420", "#f0b8a4"],
  ["#1f2a2a", "#9fdcd3"],
];

/** Цвет значка выбирается по id, чтобы у сервера он не менялся. */
export function avatarColors(id: string): { bg: string; fg: string } {
  let h = 0;
  for (const c of id) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  const [bg, fg] = AVATAR_COLORS[h % AVATAR_COLORS.length];
  return { bg, fg };
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    document.body.appendChild(area);
    area.select();
    const ok = document.execCommand("copy");
    area.remove();
    return ok;
  }
}
