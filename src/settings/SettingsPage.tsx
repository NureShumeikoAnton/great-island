import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import { api, type AppInfo, type Display, type Reminder, type Settings, type UpdateView } from "../api";
import { Glyph } from "./icons";
import { Navigator, type Route } from "./Navigator";
import { AboutPage, EnergyPage, LanguagePage } from "./pages/General";
import { IslandPage, MonitorPage, PlayerPage, RecordPage, ShotsPage } from "./pages/Features";
import { JournalPage } from "./pages/Journal";
import { MainPage } from "./pages/Main";
import { COLORS, ReminderPage, RemindersPage, SoundPage } from "./pages/Reminders";
import { SettingsContext, type SettingsState } from "./state";
import "./settings.css";

/** Changes are written this long after the last one: a slider or typing is one write, not dozens. */
const SAVE_AFTER_MS = 300;

export function SettingsPage() {
  const { t } = useTranslation();
  const [settings, setSettings] = useState<Settings | null>(null);
  const current = useRef<Settings | null>(null);
  const unsaved = useRef(false);
  const timer = useRef(0);
  const [defaultShots, setDefaultShots] = useState("");
  const [defaultRecordings, setDefaultRecordings] = useState("");
  const [displays, setDisplays] = useState<Display[]>([]);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [updating, setUpdating] = useState<UpdateView | null>(null);
  const [problemsToday, setProblemsToday] = useState(0);
  const [toastText, setToastText] = useState({ text: "", ok: true, at: 0 });

  const toast = useCallback((text: string, ok = true) => setToastText({ text, ok, at: Date.now() }), []);

  useEffect(() => {
    void api.getSettings().then((loaded) => {
      current.current = loaded;
      setSettings(loaded);
    });
    void api.defaultScreenshotDir().then(setDefaultShots);
    void api.defaultRecordingsDir().then(setDefaultRecordings);
    void api.displays().then(setDisplays);
    void api.appInfo().then(setInfo);
    const today = () => {
      const day = new Date().toLocaleDateString("sv");
      void api
        .readJournal(true)
        .then((list) => setProblemsToday(list.filter((e) => e.mark === "error" && e.at.startsWith(day)).length))
        .catch(() => setProblemsToday(0));
    };
    today();
    const poll = () => void api.updateStatus().then(setUpdating);
    poll();
    const ids = [window.setInterval(poll, 700), window.setInterval(today, 10_000)];
    return () => ids.forEach((id) => window.clearInterval(id));
  }, []);

  const flush = useCallback(async () => {
    window.clearTimeout(timer.current);
    if (!unsaved.current || !current.current) return;
    unsaved.current = false;
    try {
      await api.saveSettings(current.current);
    } catch (e) {
      toast(t("settings.save_failed", { error: String(e) }), false);
    }
  }, [t, toast]);

  // Closing must not lose the last change still waiting for its write.
  useEffect(() => {
    const unlisten = getCurrentWindow().onCloseRequested(() => flush());
    return () => void unlisten.then((stop) => stop());
  }, [flush]);

  const state = useMemo<SettingsState | null>(() => {
    if (!settings) return null;
    const update = (patch: Partial<Settings>) => {
      const next = { ...current.current!, ...patch };
      current.current = next;
      unsaved.current = true;
      setSettings(next);
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => void flush(), SAVE_AFTER_MS);
    };
    const reminders = () => current.current!.reminders;
    return {
      settings,
      update,
      updateReminder: (id: string, patch: Partial<Reminder>) =>
        update({ reminders: reminders().map((r) => (r.id === id ? { ...r, ...patch } : r)) }),
      removeReminder: (id: string) => update({ reminders: reminders().filter((r) => r.id !== id) }),
      addReminder: () => {
        const id = crypto.randomUUID();
        const list = reminders();
        update({
          reminders: [
            ...list,
            {
              id,
              name: t("reminders.new_name"),
              color: COLORS[(list.length + 2) % COLORS.length],
              tone: "bell",
              intervalMin: 45,
              durationSec: 30,
              enabled: true,
              resetOnIdle: true,
            },
          ],
        });
        return id;
      },
      info,
      updating,
      displays,
      defaultShots,
      defaultRecordings,
      problemsToday,
      journalCleared: () => setProblemsToday(0),
      toast,
    };
  }, [settings, info, updating, displays, defaultShots, defaultRecordings, problemsToday, toast, flush, t]);

  if (!state) return null;

  const render = (route: Route) => {
    const reminder = state.settings.reminders.find((r) => r.id === route.param);
    switch (route.id) {
      case "main":
        return { title: t("settings.title"), body: <MainPage /> };
      case "reminders":
        return { title: t("page.reminders"), body: <RemindersPage /> };
      case "reminder":
        return { title: reminder?.name ?? "", body: <ReminderPage id={route.param ?? ""} /> };
      case "sound":
        return { title: t("page.sound"), body: <SoundPage id={route.param ?? ""} /> };
      case "island":
        return { title: t("page.island"), body: <IslandPage /> };
      case "monitor":
        return { title: t("page.monitor"), body: <MonitorPage /> };
      case "player":
        return { title: t("page.player"), body: <PlayerPage /> };
      case "shots":
        return { title: t("page.shots"), body: <ShotsPage /> };
      case "record":
        return { title: t("page.record"), body: <RecordPage /> };
      case "energy":
        return { title: t("page.energy"), body: <EnergyPage /> };
      case "language":
        return { title: t("page.language"), body: <LanguagePage /> };
      case "about":
        return { title: t("page.about"), body: <AboutPage /> };
      case "journal":
        return { title: t("page.journal"), body: <JournalPage /> };
    }
  };

  return (
    <SettingsContext.Provider value={state}>
      <main className="settings">
        <Navigator render={render} />
        <Toast {...toastText} />
      </main>
    </SettingsContext.Provider>
  );
}

function Toast({ text, ok, at }: { text: string; ok: boolean; at: number }) {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    if (!at) return;
    setShown(true);
    const id = window.setTimeout(() => setShown(false), 1800);
    return () => window.clearTimeout(id);
  }, [at]);
  return (
    <div className={`toast ${shown ? "on" : ""}`} role="status">
      {ok && <Glyph name="check" size={14} />}
      {text}
    </div>
  );
}
