import { useEffect, useRef } from "react";

import { AppSettingsDialog } from "./components/AppSettingsDialog";
import { Sidebar } from "./components/Sidebar";
import { Toasts } from "./components/Toasts";
import { UpdateDialog } from "./components/UpdateDialog";
import { Spinner } from "./components/ui";
import { inTauri } from "./lib/api";
import { useHearth } from "./lib/store";
import { ServerView } from "./screens/server/ServerView";
import { Welcome } from "./screens/Welcome";
import { Wizard } from "./screens/wizard/Wizard";

export default function App() {
  const view = useHearth((s) => s.view);
  const init = useHearth((s) => s.init);

  const mainRef = useRef<HTMLElement>(null);
  const viewKey = view.kind === "server" ? `server:${view.id}:${view.tab}` : view.kind;

  useEffect(() => {
    void init();
  }, [init]);

  // Новый экран всегда открывается с начала, а не с места прокрутки прошлого.
  useEffect(() => {
    mainRef.current?.scrollTo(0, 0);
  }, [viewKey]);

  return (
    <div className="h-full flex bg-bg text-ink">
      <Sidebar />
      <main ref={mainRef} className="flex-1 min-w-0 overflow-y-auto">
        {view.kind === "loading" && (
          <div className="h-full flex items-center justify-center">
            <Spinner />
          </div>
        )}
        {view.kind === "welcome" && <Welcome />}
        {view.kind === "wizard" && <Wizard />}
        {view.kind === "server" && <ServerView key={view.id} id={view.id} tab={view.tab} />}
      </main>
      <AppSettingsDialog />
      <UpdateDialog />
      <Toasts />
      {!inTauri && (
        <div className="fixed top-2 right-3 z-40 px-2.5 py-1 rounded-full bg-warn-bg text-warn text-xs font-semibold">
          Превью в браузере: данные выдуманы
        </div>
      )}
    </div>
  );
}
