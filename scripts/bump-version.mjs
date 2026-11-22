// Меняет версию Hearth во всех местах сразу: package.json, Cargo.toml, tauri.conf.json.
// Использование: node scripts/bump-version.mjs 0.2.0

import { execSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

const next = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(next ?? "")) {
  console.error("Укажите версию в формате X.Y.Z, например: node scripts/bump-version.mjs 0.2.0");
  process.exit(1);
}

const pkgPath = "package.json";
const pkg = JSON.parse(readFileSync(pkgPath, "utf8"));
const current = pkg.version;

const parse = (v) => v.split(".").map(Number);
const [a, b] = [parse(current), parse(next)];
const newer = b[0] > a[0] || (b[0] === a[0] && (b[1] > a[1] || (b[1] === a[1] && b[2] > a[2])));
if (!newer) {
  console.error(`Новая версия ${next} должна быть больше текущей ${current}: иначе обновление не придёт.`);
  process.exit(1);
}

pkg.version = next;
writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");

const cargoPath = "src-tauri/Cargo.toml";
const cargo = readFileSync(cargoPath, "utf8");
writeFileSync(cargoPath, cargo.replace(/^version = "[^"]+"/m, `version = "${next}"`));

const confPath = "src-tauri/tauri.conf.json";
const conf = JSON.parse(readFileSync(confPath, "utf8"));
conf.version = next;
writeFileSync(confPath, JSON.stringify(conf, null, 2) + "\n");

// Обновляем версию самого пакета в Cargo.lock, зависимости не трогаем.
execSync("cargo update --workspace", { cwd: "src-tauri", stdio: "inherit" });

console.log(`Версия: ${current} -> ${next}`);
