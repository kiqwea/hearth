import { Flame, Plus } from "lucide-react";

import { Button } from "../components/ui";
import { t } from "../i18n";
import { useHearth } from "../lib/store";

export function Welcome() {
  const navigate = useHearth((s) => s.navigate);
  return (
    <div className="min-h-full flex items-center justify-center p-8">
      <div className="max-w-[520px] flex flex-col items-center text-center gap-5">
        <span className="w-20 h-20 rounded-3xl bg-accent-soft flex items-center justify-center">
          <Flame size={44} strokeWidth={1.6} className="text-accent" aria-hidden="true" />
        </span>
        <h1 className="m-0 text-[28px] font-bold tracking-tight">{t.welcome.title}</h1>
        <p className="m-0 text-soft leading-relaxed">{t.welcome.text}</p>
        <Button variant="primary" onClick={() => navigate({ kind: "wizard" })}>
          <Plus size={18} strokeWidth={2.2} aria-hidden="true" />
          {t.welcome.create}
        </Button>
      </div>
    </div>
  );
}
