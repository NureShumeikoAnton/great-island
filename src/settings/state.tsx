
import { createContext, useContext, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, type AppInfo, type Display, type Hotkey, type Reminder, type Settings, type UpdateView } from "../api";

export interface SettingsState {
  settings: Settings;
  /** Applies at once; the file is written a moment later. */
  update: (patch: Partial<Settings>) => void;
  updateReminder: (id: string, patch: Partial<Reminder>) => void;
  removeReminder: (id: string) => void;
  /** Returns the new reminder's id. */
  addReminder: () => string;
  info: AppInfo | null;
  /** Where the update stands: the island keeps it, this window only shows it. */
  updating: UpdateView | null;
  displays: Display[];
  defaultShots: string;
  defaultRecordings: string;
  /** Critical problems in the journal today. */
  problemsToday: number;
  journalCleared: () => void;
  /** `ok` false: something did not work, the toast has no check mark. */
  toast: (text: string, ok?: boolean) => void;
}

export const SettingsContext = createContext<SettingsState | null>(null);

export function useSettings(): SettingsState {
  const state = useContext(SettingsContext);
  if (!state) throw new Error("useSettings outside the settings window");
  return state;
}

/** The key as Windows names it, "Ctrl + Shift + S"; empty for none. */
export function useKeyLabel(key: Hotkey | null): string {
  const [label, setLabel] = useState("");
  useEffect(() => {
    if (!key) return setLabel("");
    let live = true;
    void api.checkHotkey(key).then((check) => live && setLabel(check.label));
    return () => {
      live = false;
    };
  }, [key]);
  return label;
}

type Translate = ReturnType<typeof useTranslation>["t"];

const dayOf = (date: Date) => new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();

/** -1 before yesterday, 0 for yesterday, 1 for today. */
function recency(date: Date): number {
  const today = dayOf(new Date());
  const day = dayOf(date);
  return day === today ? 1 : day === today - 86_400_000 ? 0 : -1;
}

export function dayTitle(date: Date, t: Translate, lang: string): string {
  switch (recency(date)) {
    case 1:
      return t("when.today_title");
    case 0:
      return t("when.yesterday_title");
    default:
      return date.toLocaleDateString(lang, { day: "numeric", month: "long" });
  }
}

/** "today at 10:28", "yesterday at…", "15 November at…". */
export function when(ms: number, t: Translate, lang: string): string {
  const date = new Date(ms);
  const time = date.toLocaleTimeString(lang, { hour: "2-digit", minute: "2-digit" });
  switch (recency(date)) {
    case 1:
      return t("when.today", { time });
    case 0:
      return t("when.yesterday", { time });
    default:
      return t("when.date", { date: date.toLocaleDateString(lang, { day: "numeric", month: "long" }), time });
  }
}

/** One line on where the update stands, for the card on top and for About. */
export function updateLine(view: UpdateView | null, t: Translate, lang: string): string {
  if (!view) return "";
  if (!view.running) return t("about.no_island");
  const { status, checkedAt } = view;
  switch (status.phase) {
    case "checking":
      return t("about.checking");
    case "available":
      return status.offer
        ? t(status.offer.skipped ? "about.available_skipped" : "about.available", { version: status.offer.latest })
        : "";
    case "failed":
    case "installFailed":
      return t("about.failed_short");
    case "downloading":
    case "verifying":
    case "installing":
      return t("about.downloading");
    case "upToDate":
      return checkedAt ? t("about.up_to_date_at", { when: when(checkedAt, t, lang) }) : t("about.latest");
    default:
      return checkedAt ? t("about.checked_at", { when: when(checkedAt, t, lang) }) : "";
  }
}
