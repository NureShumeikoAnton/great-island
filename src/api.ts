// Types and calls for the settings window's Rust side. Fields mirror the serde structs in src-tauri.

import { invoke } from "@tauri-apps/api/core";

/** In the order the sound list shows them; "none" is silence. */
export const TONES = ["none", "bell", "soft", "drop", "kalimba", "breath", "bowl", "bubble", "chime"] as const;

export type Tone = (typeof TONES)[number];

export interface Reminder {
  id: string;
  name: string;
  color: string;
  tone: Tone;
  intervalMin: number;
  durationSec: number;
  enabled: boolean;
  resetOnIdle: boolean;
}

export interface Settings {
  reminders: Reminder[];
  volume: number;
  idleMin: number;
  showIsland: boolean;
  /** A monitor's device name; empty = the island follows the cursor. */
  islandMonitor: string;
  fullscreen: Fullscreen;
  player: boolean;
  announceTracks: boolean;
  /** Minutes a "playing" player may stay silent before it is dropped from the island; 0 = never. */
  silentDropMin: number;
  liveWhenDimmed: boolean;
  keepScreenOn: boolean;
  /** Decorative animations: for now the wave when an area selection opens. */
  animations: boolean;
  /** Corner marks around the area of a recording. */
  areaMarks: boolean;
  /** Empty = the system Screenshots folder. */
  screenshotDir: string;
  screenshotKey: Hotkey | null;
  /** A screenshot of an area picked first. */
  areaKey: Hotkey | null;
  viewerBar: ViewerBar;
  /** Every screenshot is in the clipboard at once; the card looks as without it. */
  shotClipboard: boolean;
  /** Empty = the default recordings folder. */
  recordingsDir: string;
  /** Sound, countdown, quality and the rest are the island's pre-start panel: not edited here. */
  recordKey: Hotkey | null;
  autostart: boolean;
  /** Look for a new version by itself; the manual check stays. */
  checkUpdates: boolean;
  language: Language;
  /** System notifications: low and critical charge. */
  notifyLowCharge: boolean;
  /** Plugged in, unplugged, charged, slow charging. */
  notifyCharging: boolean;
  /** Fast drain on battery, high load without it. */
  notifyUsage: boolean;
  /** A download coming and the files that arrived in Downloads. */
  notifyDownloads: boolean;
  /** Bluetooth headphones and speakers connected or disconnected. */
  notifyDevices: boolean;
  /** A Bluetooth device's low battery. */
  notifyDeviceBattery: boolean;
  /** Windows moved the sound or the microphone to another device. */
  notifyAudioSwitch: boolean;
  /** The control center's bottom line: the charge in numbers. */
  batteryPercent: boolean;
  clockSeconds: boolean;
}

/** "auto" follows Windows. The island reads it at start: a change applies after a restart. */
export interface Display {
  /** The device name, like "\\.\DISPLAY2": what the setting stores. */
  name: string;
  primary: boolean;
  width: number;
  height: number;
}

export type Language = "auto" | "en" | "ru";

export type Fullscreen = "show" | "alerts" | "hide";

export type ViewerBar = "strip" | "floating";

/** Keyed by physical key (KeyboardEvent.code), so the keyboard layout doesn't matter. */
export interface Hotkey {
  code: string;
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  win: boolean;
}

export const PRINT_SCREEN: Hotkey = { code: "PrintScreen", ctrl: false, alt: false, shift: false, win: false };

export interface KeyCheck {
  label: string;
  problem: "unknown" | "tooPlain" | "busy" | null;
  /** True when this is plain PrtSc and Windows Snipping Tool currently owns it. */
  snipping: boolean;
}

export interface AppInfo {
  version: string;
  license: string;
  /** The Windows build number. */
  windows: number;
  /** The computer has a battery: the battery notifications apply. */
  battery: boolean;
}

/** One journal entry; a run of the same one comes as one with `times`. */
export interface JournalEntry {
  /** Local time, "2026-11-17 10:28:03". */
  at: string;
  mark: "info" | "warn" | "error";
  /** "UPD-CHECK/MISSING" for a problem the island reported. */
  code: string | null;
  title: string;
  reason: string | null;
  detail: string;
  times: number;
  /** The newest line as it is in the file. */
  line: string;
}

/** Where an update stands: the island writes it, every window shows it. */
export type UpdatePhase =
  | "idle"
  | "checking"
  | "upToDate"
  | "failed"
  | "available"
  | "downloading"
  | "verifying"
  | "installing"
  | "installFailed";

export interface OfferView {
  current: string;
  latest: string;
  notes: { en: string; ru: string };
  canSkip: boolean;
  canPostpone: boolean;
  /** The user passed this version over once: it is offered because they asked. */
  skipped: boolean;
  size: number;
}

export interface UpdateStatus {
  phase: UpdatePhase;
  manual: boolean;
  error: string | null;
  offer: OfferView | null;
  done: number;
  total: number;
  at: number;
}

export interface UpdateView {
  /** An island is there to take commands. */
  running: boolean;
  status: UpdateStatus;
  /** Unix ms of the last check that got an answer. */
  checkedAt: number | null;
}

export type UpdateCommand = "check" | "openWindow" | "install" | "postpone" | "skip";

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  currentLanguage: () => invoke<string>("current_language"),
  /** True when the choice gives another language than the one in use (the island changes at its restart). */
  languagePending: (choice: Language) => invoke<boolean>("language_pending", { choice }),
  /** `end`: the sound of a break that is over. */
  previewSound: (tone: Tone, volume: number, end = false) => invoke<void>("preview_sound", { tone, volume, end }),
  defaultScreenshotDir: () => invoke<string>("default_screenshot_dir"),
  defaultRecordingsDir: () => invoke<string>("default_recordings_dir"),
  /** null = the user cancelled. */
  pickFolder: (current: string, recordings = false) =>
    invoke<string | null>("pick_folder", { current, recordings }),
  checkHotkey: (key: Hotkey) => invoke<KeyCheck>("check_hotkey", { key }),
  /** false = the setting couldn't be written. */
  releasePrintScreen: () => invoke<boolean>("release_print_screen"),
  displays: () => invoke<Display[]>("list_displays"),
  openJournal: () => invoke<boolean>("open_journal"),
  /** Newest first. Rejects with the reason the file could not be read. */
  readJournal: (problemsOnly: boolean) => invoke<JournalEntry[]>("read_journal", { problemsOnly }),
  /** Rejects with the reason the file could not be emptied. */
  clearJournal: () => invoke<void>("clear_journal"),
  appInfo: () => invoke<AppInfo>("app_info"),
  openProjectPage: (page: "home" | "license" | "changes") => invoke<boolean>("open_project_page", { page }),
  // Updates: the island does the work; "About" reads the status and sends commands.
  updateStatus: () => invoke<UpdateView>("update_status"),
  /** False: there is no island to hear it. */
  updateCommand: (name: UpdateCommand) => invoke<boolean>("update_command", { name }),
};
