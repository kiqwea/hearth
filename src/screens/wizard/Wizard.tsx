import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowRight, Check, Info, ShieldCheck, X } from "lucide-react";

import type { Difficulty } from "../../bindings/Difficulty";
import type { GameMode } from "../../bindings/GameMode";
import type { GameVersion } from "../../bindings/GameVersion";
import type { Loader } from "../../bindings/Loader";
import { Button, Card, Field, inputClass, ProgressBar, Segmented, Spinner, Switch } from "../../components/ui";
import { t } from "../../i18n";
import { api, errorMessage, openExternal } from "../../lib/api";
import { authPluginName, formatMb, gb, loaderName, skinPluginName } from "../../lib/format";
import { useHearth } from "../../lib/store";

interface FormState {
  loader: Loader;
  version: string;
  showUnstable: boolean;
  name: string;
  memoryMb: number;
  gameMode: GameMode;
  difficulty: Difficulty;
  maxPlayers: number;
  seed: string;
  port: number;
  /** Вход с пиратских клиентов (TLauncher и др.). */
  pirateMode: boolean;
  /** Защита ников паролем в пиратском режиме. */
  nickProtection: boolean;
  /** Скины Ely.by в пиратском режиме. */
  elySkins: boolean;
}

const LOADERS: Loader[] = ["paper", "vanilla", "fabric", "purpur", "neoforge", "forge"];

export function Wizard() {
  const system = useHearth((s) => s.system);
  const [step, setStep] = useState<1 | 2>(1);
  const [form, setForm] = useState<FormState>(() => ({
    loader: "paper",
    version: "",
    showUnstable: false,
    name: t.wizard.defaultName,
    memoryMb: system?.recommendedMemoryMb ?? 4096,
    gameMode: "survival",
    difficulty: "normal",
    maxPlayers: 10,
    seed: "",
    port: 25565,
    pirateMode: false,
    nickProtection: true,
    elySkins: true,
  }));
  const [createdId, setCreatedId] = useState<string | null>(null);

  return (
    <div className="px-8 py-7">
      <div className="w-full max-w-[820px] flex flex-col gap-[22px]">
        <header className="flex flex-col gap-3.5">
          <h1 className="m-0 text-[28px] font-bold tracking-tight">{t.wizard.title}</h1>
          <Steps step={step} />
        </header>
        {step === 1 ? (
          <FormStep form={form} setForm={setForm} onNext={() => setStep(2)} />
        ) : (
          <InstallStep form={form} createdId={createdId} setCreatedId={setCreatedId} onBack={() => setStep(1)} />
        )}
      </div>
    </div>
  );
}

function Steps({ step }: { step: 1 | 2 }) {
  const items = [t.wizard.stepServer, t.wizard.stepInstall];
  return (
    <ol aria-label="Шаги" className="m-0 p-0 list-none flex flex-wrap gap-2">
      {items.map((label, i) => {
        const n = (i + 1) as 1 | 2;
        const done = n < step;
        const active = n === step;
        return (
          <li
            key={label}
            aria-current={active ? "step" : undefined}
            className={`flex items-center gap-2 py-1.5 pl-1.5 pr-3 rounded-full ${
              active ? "bg-accent-soft text-accent-text font-semibold" : "bg-[#1e1a16] text-muted"
            }`}
          >
            <span
              className={`w-6 h-6 rounded-full flex items-center justify-center text-xs font-bold ${
                done ? "bg-[#2f4a39] text-[#a8efc6]" : active ? "bg-accent text-accent-ink" : "bg-[#2e2822] text-soft"
              }`}
            >
              {done ? <Check size={14} strokeWidth={3} aria-hidden="true" /> : n}
            </span>
            {label}
          </li>
        );
      })}
    </ol>
  );
}

function FormStep({
  form,
  setForm,
  onNext,
}: {
  form: FormState;
  setForm: (f: (prev: FormState) => FormState) => void;
  onNext: () => void;
}) {
  const system = useHearth((s) => s.system);
  const [versions, setVersions] = useState<GameVersion[] | null>(null);
  const [versionsError, setVersionsError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const update = <K extends keyof FormState>(key: K, value: FormState[K]) => setForm((f) => ({ ...f, [key]: value }));

  useEffect(() => {
    let cancelled = false;
    setVersions(null);
    setVersionsError(null);
    api.listVersions(form.loader, form.showUnstable).then(
      (list) => {
        if (cancelled) return;
        setVersions(list);
        setForm((f) => {
          if (list.some((v) => v.id === f.version)) return f;
          const latest = list.find((v) => v.stable) ?? list[0];
          return { ...f, version: latest?.id ?? "" };
        });
      },
      (e) => !cancelled && setVersionsError(errorMessage(e)),
    );
    return () => {
      cancelled = true;
    };
  }, [form.loader, form.showUnstable, reload, setForm]);

  const totalMb = system?.totalMemoryMb ?? 0;
  const maxGb = totalMb > 0 ? Math.max(2, Math.min(32, Math.floor((totalMb * 0.75) / 1024))) : 16;
  const latestStable = versions?.find((v) => v.stable)?.id;
  const nameOk = form.name.trim().length > 0 && form.name.trim().length <= 64;
  const portOk = form.port >= 1024 && form.port <= 65535;
  const canContinue = nameOk && portOk && form.version !== "" && form.maxPlayers >= 1;

  return (
    <>
      <section aria-label={t.wizard.edition} className="flex flex-col gap-2.5">
        <h2 className="m-0 text-base font-semibold">{t.wizard.edition}</h2>
        <div className="flex flex-wrap gap-2.5">
          <button type="button" aria-pressed="true" className="flex-[1_1_240px] flex flex-col gap-1 p-3.5 px-4 rounded-xl border-2 border-accent bg-[#221a13] text-left">
            <span className="font-bold">{t.wizard.java}</span>
            <span className="text-[13px] text-soft">{t.wizard.javaAbout}</span>
          </button>
          <button type="button" aria-pressed="false" disabled className="flex-[1_1_240px] flex flex-col gap-1 p-3.5 px-4 rounded-xl border border-line bg-[#181512] text-faint text-left">
            <span className="font-bold">
              {t.wizard.bedrock}
              <span className="font-medium text-xs px-2 py-0.5 rounded-full bg-chip ml-1.5">{t.app.soon}</span>
            </span>
            <span className="text-[13px]">{t.wizard.bedrockAbout}</span>
          </button>
        </div>
      </section>

      <section aria-label={t.wizard.core} className="flex flex-col gap-2.5">
        <h2 className="m-0 text-base font-semibold">{t.wizard.core}</h2>
        <div className="grid gap-2.5 grid-cols-[repeat(auto-fit,minmax(300px,1fr))]">
          {LOADERS.map((loader) => {
            const active = form.loader === loader;
            return (
              <button
                key={loader}
                type="button"
                aria-pressed={active}
                onClick={() => update("loader", loader)}
                className={`flex flex-col gap-1.5 p-3.5 px-4 rounded-xl text-left transition ${
                  active ? "border-2 border-accent bg-[#221a13]" : "border border-line bg-card hover:bg-[#211c18]"
                }`}
              >
                <span className="flex items-center justify-between gap-2">
                  <span className="font-bold">{loaderName(loader)}</span>
                  {loader === "paper" && (
                    <span className="text-xs font-semibold px-2 py-0.5 rounded-full bg-accent text-accent-ink">{t.loader.recommended}</span>
                  )}
                </span>
                <span className={`text-[13px] ${active ? "text-soft" : "text-muted"}`}>{t.loader[loader].about}</span>
              </button>
            );
          })}
        </div>
      </section>

      <div className="flex flex-wrap gap-4">
        <Field label={t.wizard.version} className="flex-[1_1_260px]">
          {(id) => (
            <>
              {versionsError ? (
                <div className="flex items-center gap-3 h-11 px-3 rounded-[10px] border border-[#5a2a24] bg-err-bg text-err text-[13px]">
                  <span className="flex-1">{t.wizard.versionsFailed}</span>
                  <Button size="sm" onClick={() => setReload((r) => r + 1)}>
                    {t.actions.retry}
                  </Button>
                </div>
              ) : (
                <select
                  id={id}
                  value={form.version}
                  disabled={!versions}
                  onChange={(e) => update("version", e.target.value)}
                  className={inputClass}
                >
                  {!versions && <option>{t.wizard.loadingVersions}</option>}
                  {versions?.map((v) => (
                    <option key={v.id} value={v.id}>
                      {v.id}
                      {v.id === latestStable ? ` — ${t.wizard.latest}` : ""}
                    </option>
                  ))}
                </select>
              )}
              <label className="flex items-center gap-2 text-muted text-[13px]">
                <input
                  type="checkbox"
                  checked={form.showUnstable}
                  onChange={(e) => update("showUnstable", e.target.checked)}
                  className="w-[18px] h-[18px]"
                />
                {t.wizard.showUnstable}
              </label>
            </>
          )}
        </Field>
        <Field label={t.wizard.name} hint={t.wizard.nameHint} className="flex-[1_1_260px]">
          {(id) => (
            <input id={id} value={form.name} maxLength={64} onChange={(e) => update("name", e.target.value)} className={inputClass} />
          )}
        </Field>
      </div>

      <Card label={t.wizard.memory} className="flex flex-col gap-2.5 p-4 px-[18px]">
        <div className="flex flex-wrap items-baseline justify-between gap-2">
          <label htmlFor="wizard-ram" className="font-semibold text-base">
            {t.wizard.memory}
          </label>
          <span className="text-[22px] font-bold">{String(gb(form.memoryMb)).replace(".", ",")} ГБ</span>
        </div>
        <input
          id="wizard-ram"
          type="range"
          min={1}
          max={maxGb}
          step={1}
          value={Math.round(form.memoryMb / 1024)}
          onChange={(e) => update("memoryMb", Number(e.target.value) * 1024)}
          className="w-full"
        />
        <div className="flex justify-between text-xs text-muted">
          <span>1 ГБ</span>
          <span>{maxGb} ГБ</span>
        </div>
        {system && system.totalMemoryMb > 0 && (
          <p className="m-0 flex gap-2.5 items-start text-soft text-[13px]">
            <Info size={18} className="text-accent shrink-0 mt-px" aria-hidden="true" />
            {t.wizard.memoryHint(Math.round(system.totalMemoryMb / 1024), gb(system.recommendedMemoryMb))}
          </p>
        )}
      </Card>

      <section aria-label={t.wizard.world} className="flex flex-col gap-4">
        <h2 className="m-0 text-base font-semibold">{t.wizard.world}</h2>
        <div className="flex flex-col gap-2">
          <span className="font-semibold">{t.wizard.gameMode}</span>
          <Segmented<GameMode>
            label={t.wizard.gameMode}
            value={form.gameMode}
            onChange={(v) => update("gameMode", v)}
            options={(["survival", "creative", "adventure"] as GameMode[]).map((v) => ({ value: v, label: t.gameMode[v] }))}
          />
        </div>
        <div className="flex flex-col gap-2">
          <span className="font-semibold">{t.wizard.difficulty}</span>
          <Segmented<Difficulty>
            label={t.wizard.difficulty}
            value={form.difficulty}
            onChange={(v) => update("difficulty", v)}
            options={(["peaceful", "easy", "normal", "hard"] as Difficulty[]).map((v) => ({ value: v, label: t.difficulty[v] }))}
          />
        </div>
        <div className="flex flex-wrap gap-4">
          <Field label={t.wizard.slots} className="flex-[1_1_160px]">
            {(id) => (
              <input
                id={id}
                type="number"
                min={1}
                max={1000}
                value={form.maxPlayers}
                onChange={(e) => update("maxPlayers", Math.max(1, Math.min(1000, Number(e.target.value) || 1)))}
                className={inputClass}
              />
            )}
          </Field>
          <Field label={t.wizard.seed} className="flex-[3_1_300px]">
            {(id) => (
              <input
                id={id}
                value={form.seed}
                placeholder={t.wizard.seedPlaceholder}
                onChange={(e) => update("seed", e.target.value)}
                className={inputClass}
              />
            )}
          </Field>
        </div>
      </section>

      <section aria-label={t.wizard.players} className="flex flex-col gap-3">
        <h2 className="m-0 text-base font-semibold">{t.wizard.players}</h2>
        <ToggleRow
          label={t.pirate.label}
          hint={t.wizard.pirateHint}
          checked={form.pirateMode}
          onChange={(v) => update("pirateMode", v)}
        />
        {form.pirateMode &&
          (!authPluginName(form.loader) ? (
            <p className="m-0 flex items-start gap-2.5 px-3.5 py-3 rounded-[10px] bg-err-bg text-err text-[13px]">
              <Info size={16} className="shrink-0 mt-0.5" aria-hidden="true" />
              {t.wizard.vanillaNoProtection(loaderName(form.loader))}
            </p>
          ) : (
            <ToggleRow
              label={t.protection.label}
              hint={form.nickProtection ? t.protection.onHint : t.protection.offHint}
              checked={form.nickProtection}
              onChange={(v) => update("nickProtection", v)}
              warn={!form.nickProtection}
            />
          ))}
        {form.pirateMode && skinPluginName(form.loader) && (
          <ToggleRow
            label={t.skins.label}
            hint={form.elySkins ? t.skins.onHint : t.skins.offHint}
            checked={form.elySkins}
            onChange={(v) => update("elySkins", v)}
          />
        )}
      </section>

      <details className="bg-card border border-line rounded-xl p-3.5 px-4">
        <summary className="font-semibold">{t.wizard.advanced}</summary>
        <div className="pt-3 max-w-[240px]">
          <Field label={t.wizard.port} hint={t.wizard.portHint}>
            {(id) => (
              <input
                id={id}
                type="number"
                min={1024}
                max={65535}
                value={form.port}
                onChange={(e) => update("port", Number(e.target.value))}
                className={`${inputClass} font-mono ${portOk ? "" : "border-err"}`}
              />
            )}
          </Field>
        </div>
      </details>

      <footer className="flex flex-wrap justify-end gap-2.5 pt-2 border-t border-line">
        <Button variant="primary" disabled={!canContinue} onClick={onNext}>
          {t.actions.next}
          <ArrowRight size={16} strokeWidth={2.2} aria-hidden="true" />
        </Button>
      </footer>
    </>
  );
}

function ToggleRow({
  label,
  hint,
  checked,
  onChange,
  warn,
}: {
  label: string;
  hint: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  warn?: boolean;
}) {
  return (
    <div className="flex items-center gap-3 bg-card border border-line rounded-xl px-4 py-3">
      <span className="flex-1 flex flex-col gap-0.5">
        <span className="font-semibold">{label}</span>
        <span className={`text-[13px] ${warn ? "text-warn" : "text-muted"}`}>{hint}</span>
      </span>
      <Switch checked={checked} onChange={onChange} label={label} />
    </div>
  );
}

type StepState = "pending" | "active" | "done" | "failed";

function InstallStep({
  form,
  createdId,
  setCreatedId,
  onBack,
}: {
  form: FormState;
  createdId: string | null;
  setCreatedId: (id: string) => void;
  onBack: () => void;
}) {
  const [accepted, setAccepted] = useState(false);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const server = useHearth((s) => (createdId ? s.servers[createdId] : undefined));
  const progress = useHearth((s) => (createdId ? s.install[createdId] : undefined));
  const refreshServers = useHearth((s) => s.refreshServers);
  const openServer = useHearth((s) => s.openServer);

  const state = server?.status.state;
  const done = state === "running";
  // Последний активный шаг: при ошибке подсвечиваем именно его.
  const lastActive = useRef(0);

  async function begin() {
    setWorking(true);
    setError(null);
    try {
      let id = createdId;
      if (!id) {
        const meta = await api.createServer({
          name: form.name.trim(),
          loader: form.loader,
          gameVersion: form.version,
          memoryMb: form.memoryMb,
          port: form.port,
          gameMode: form.gameMode,
          difficulty: form.difficulty,
          maxPlayers: form.maxPlayers,
          seed: form.seed.trim() || null,
          eulaAccepted: true,
          pirateMode: form.pirateMode,
          nickProtection: form.pirateMode && form.nickProtection && authPluginName(form.loader) !== null,
          elySkins: form.pirateMode && form.elySkins && skinPluginName(form.loader) !== null,
        });
        id = meta.id;
        setCreatedId(id);
        await refreshServers();
      }
      await api.start(id);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setWorking(false);
    }
  }

  // Какой шаг сейчас идёт: по событиям установки и статусу сервера.
  const steps = useMemo(() => {
    const order = { resolving: 0, java: 1, server: 2, done: 3, failed: -1 } as const;
    let current = -1;
    if (progress) current = order[progress.stage];
    if (progress?.stage === "failed") current = lastActive.current;
    if (state === "starting") current = 3;
    if (state === "running") current = 4;
    if (current >= 0) lastActive.current = current;
    const status = (i: number): StepState => {
      if (error && i === Math.max(current, 0)) return "failed";
      if (current > i) return "done";
      if (current === i) return "active";
      return "pending";
    };
    const bytes =
      progress && progress.downloaded !== null
        ? {
            pct: progress.total ? (progress.downloaded / progress.total) * 100 : null,
            text: progress.total
              ? `${formatMb(progress.downloaded)} из ${formatMb(progress.total)}`
              : formatMb(progress.downloaded),
          }
        : null;
    return [
      { title: t.wizard.stepResolve(`${loaderName(form.loader)} ${form.version}`), state: status(0) },
      { title: progress?.stage === "java" ? progress.message : t.wizard.stepJava, about: t.wizard.stepJavaAbout, state: status(1), bytes: progress?.stage === "java" ? bytes : null },
      { title: progress?.stage === "server" ? progress.message : t.wizard.stepCore, state: status(2), bytes: progress?.stage === "server" ? bytes : null },
      { title: t.wizard.stepFirstRun, state: status(3) },
    ];
  }, [progress, state, error, form.loader, form.version]);

  const started = createdId !== null;

  return (
    <>
      <Card label={t.wizard.eulaTitle} className="flex flex-col gap-3 p-[18px] px-5">
        <h2 className="m-0 text-base font-semibold">{t.wizard.eulaTitle}</h2>
        <p className="m-0 text-soft leading-relaxed">{t.wizard.eulaText}</p>
        <a
          href="https://aka.ms/MinecraftEULA"
          onClick={(e) => {
            e.preventDefault();
            void openExternal("https://aka.ms/MinecraftEULA");
          }}
          className="font-semibold text-link hover:text-accent-text w-fit"
        >
          {t.wizard.eulaLink}
        </a>
        <label className="flex items-center gap-2.5 p-3 px-3.5 rounded-[10px] bg-raised font-semibold">
          <input
            type="checkbox"
            checked={accepted || started}
            disabled={started}
            onChange={(e) => setAccepted(e.target.checked)}
            className="w-5 h-5"
          />
          {t.wizard.eulaAccept}
        </label>
      </Card>

      {started && (
        <Card label={t.wizard.install} className="flex flex-col gap-1 p-[18px] px-5">
          <h2 className="m-0 mb-2.5 text-base font-semibold">{t.wizard.install}</h2>
          {steps.map((s) => (
            <div key={s.title} className={`flex items-start gap-3.5 py-2.5 ${s.state === "pending" ? "text-faint" : ""}`}>
              <StepIcon state={s.state} />
              <span className="flex-1 flex flex-col gap-2 pt-0.5">
                <span className="flex flex-wrap justify-between gap-2">
                  <span className="font-semibold">{s.title}</span>
                  {s.bytes && <span className="text-[13px] text-soft">{s.bytes.text}</span>}
                </span>
                {s.state === "active" && s.bytes && <ProgressBar value={s.bytes.pct} label={s.title} />}
                {s.state === "active" && s.about && <span className="text-[13px] text-muted">{s.about}</span>}
              </span>
            </div>
          ))}
          {error && (
            <div role="alert" className="mt-2 p-3 px-3.5 rounded-[10px] bg-err-bg text-err selectable">
              <b>{t.wizard.failed}:</b> {error}
            </div>
          )}
        </Card>
      )}

      {started && !done && !error && (
        <div className="flex gap-3 items-start px-4 py-3 rounded-xl bg-accent-soft text-accent-text">
          <ShieldCheck size={20} className="shrink-0 mt-0.5" aria-hidden="true" />
          <span className="flex flex-col gap-1">
            <b>{t.wizard.firewallTitle}</b>
            <span className="text-soft">{t.wizard.firewallText}</span>
          </span>
        </div>
      )}

      {started && !done && !error && (
        <p className="m-0 flex gap-2.5 items-start text-muted text-[13px]">
          <Info size={18} className="shrink-0" aria-hidden="true" />
          {t.wizard.trayHint}
        </p>
      )}

      {done && (
        <div className="flex items-center gap-3 p-4 rounded-xl bg-ok-bg text-ok-text font-semibold">
          <Check size={20} strokeWidth={2.6} aria-hidden="true" />
          {t.wizard.done}
        </div>
      )}

      <footer className="flex flex-wrap justify-between gap-2.5 pt-2 border-t border-line">
        <Button variant="ghost" onClick={onBack} disabled={started}>
          {t.actions.back}
        </Button>
        {done && createdId ? (
          <Button variant="primary" onClick={() => openServer(createdId)}>
            {t.wizard.openServer}
            <ArrowRight size={16} strokeWidth={2.2} aria-hidden="true" />
          </Button>
        ) : error ? (
          <Button variant="primary" onClick={begin} disabled={working}>
            {t.actions.retry}
          </Button>
        ) : (
          <Button variant="primary" onClick={begin} disabled={!accepted || started}>
            {t.wizard.createAndStart}
          </Button>
        )}
      </footer>
    </>
  );
}

function StepIcon({ state }: { state: StepState }) {
  if (state === "done")
    return (
      <span className="w-7 h-7 rounded-full bg-[#2f4a39] text-[#a8efc6] flex items-center justify-center shrink-0">
        <Check size={15} strokeWidth={3} aria-hidden="true" />
      </span>
    );
  if (state === "active") return <Spinner />;
  if (state === "failed")
    return (
      <span className="w-7 h-7 rounded-full bg-err-bg text-err flex items-center justify-center shrink-0">
        <X size={15} strokeWidth={3} aria-hidden="true" />
      </span>
    );
  return <span className="w-7 h-7 rounded-full border-2 border-line-strong shrink-0" />;
}
