// Types and calls for the settings window's Rust side. Fields mirror the serde structs in src-tauri.

import { invoke } from "@tauri-apps/api/core";

export type Tone = "bell" | "drop" | "soft";

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
  /** Empty = the default recordings folder. */
  recordingsDir: string;
  /** Sound, countdown, quality and the rest are the island's pre-start panel: not edited here. */
  recordKey: Hotkey | null;
  autostart: boolean;
  language: Language;
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

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  currentLanguage: () => invoke<string>("current_language"),
  /** True when the choice gives another language than the one in use (the island changes at its restart). */
  languagePending: (choice: Language) => invoke<boolean>("language_pending", { choice }),
  previewSound: (tone: Tone, volume: number) => invoke<void>("preview_sound", { tone, volume }),
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
};
