// Описание параметров server.properties для редактора настроек:
// русские названия, подсказки и тип поля. Остальные параметры доступны
// в разделе «Все параметры» как есть.

export type PropertyKind =
  | { type: "bool" }
  | { type: "int"; min: number; max: number }
  | { type: "range"; min: number; max: number; unit: string }
  | { type: "select"; options: Array<{ value: string; label: string }> }
  | { type: "text"; placeholder?: string };

export interface PropertyDef {
  key: string;
  label: string;
  hint?: string;
  kind: PropertyKind;
  default: string;
}

export interface PropertyGroup {
  title: string;
  items: PropertyDef[];
}

export const PROPERTY_GROUPS: PropertyGroup[] = [
  {
    title: "Игра",
    items: [
      {
        key: "gamemode",
        label: "Режим игры",
        hint: "Режим для новых игроков",
        default: "survival",
        kind: {
          type: "select",
          options: [
            { value: "survival", label: "Выживание" },
            { value: "creative", label: "Творческий" },
            { value: "adventure", label: "Приключение" },
            { value: "spectator", label: "Наблюдатель" },
          ],
        },
      },
      {
        key: "difficulty",
        label: "Сложность",
        hint: "На «Мирной» не появляются враждебные мобы",
        default: "easy",
        kind: {
          type: "select",
          options: [
            { value: "peaceful", label: "Мирная" },
            { value: "easy", label: "Лёгкая" },
            { value: "normal", label: "Нормальная" },
            { value: "hard", label: "Сложная" },
          ],
        },
      },
      { key: "pvp", label: "PvP", hint: "Игроки могут наносить урон друг другу", default: "true", kind: { type: "bool" } },
      {
        key: "hardcore",
        label: "Хардкор",
        hint: "Одна жизнь: после смерти игрок становится наблюдателем",
        default: "false",
        kind: { type: "bool" },
      },
      {
        key: "max-players",
        label: "Слоты для игроков",
        hint: "Сколько человек может быть на сервере одновременно",
        default: "20",
        kind: { type: "int", min: 1, max: 1000 },
      },
      {
        key: "view-distance",
        label: "Дальность прорисовки",
        hint: "Чем меньше, тем легче серверу. Для слабого ПК хватит 6–8",
        default: "10",
        kind: { type: "range", min: 3, max: 32, unit: "чанков" },
      },
      {
        key: "simulation-distance",
        label: "Дальность симуляции",
        hint: "На каком расстоянии от игроков растут растения и двигаются мобы",
        default: "10",
        kind: { type: "range", min: 3, max: 32, unit: "чанков" },
      },
      {
        key: "allow-flight",
        label: "Разрешить полёт",
        hint: "Без этого сервер кикает игроков, если думает, что они летают (полезно для модов с джетпаками)",
        default: "false",
        kind: { type: "bool" },
      },
      {
        key: "force-gamemode",
        label: "Режим при каждом входе",
        hint: "Игроки всегда заходят в режиме сервера, даже если он у них менялся",
        default: "false",
        kind: { type: "bool" },
      },
    ],
  },
  {
    title: "Мир",
    items: [
      {
        key: "level-seed",
        label: "Сид мира",
        hint: "Влияет только на новый мир. У уже созданного мира сид не меняется",
        default: "",
        kind: { type: "text", placeholder: "Случайный" },
      },
      {
        key: "generate-structures",
        label: "Генерировать постройки",
        hint: "Деревни, храмы, крепости в новых чанках",
        default: "true",
        kind: { type: "bool" },
      },
      { key: "allow-nether", label: "Нижний мир", hint: "Можно попасть в Незер через портал", default: "true", kind: { type: "bool" } },
      {
        key: "spawn-protection",
        label: "Защита спавна",
        hint: "Радиус в блоках вокруг точки появления, где строить могут только операторы. 0 — без защиты",
        default: "16",
        kind: { type: "int", min: 0, max: 1000 },
      },
    ],
  },
  {
    title: "Доступ",
    items: [
      {
        key: "white-list",
        label: "Белый список",
        hint: "Заходить могут только игроки из списка (команда whitelist add <ник>)",
        default: "false",
        kind: { type: "bool" },
      },
      {
        key: "enforce-whitelist",
        label: "Выгонять не из списка",
        hint: "При включении белого списка сразу кикнуть тех, кого в нём нет",
        default: "false",
        kind: { type: "bool" },
      },
      {
        key: "server-port",
        label: "Порт",
        hint: "Меняйте, только если 25565 занят другой программой",
        default: "25565",
        kind: { type: "int", min: 1024, max: 65535 },
      },
    ],
  },
];

/** Параметры, которые редактируются в других местах (motd — в «Основном»). */
export const HANDLED_ELSEWHERE = new Set(["motd"]);

export const KNOWN_KEYS = new Set(PROPERTY_GROUPS.flatMap((g) => g.items.map((i) => i.key)));
