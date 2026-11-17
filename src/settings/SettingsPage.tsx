import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import {
  api,
  type AppInfo,
  type Display,
  type Fullscreen,
  type Language,
  type Reminder,
  type Settings,
  type Tone,
  type UpdateView,
  type ViewerBar,
} from "../api";
import { HotkeyField } from "./HotkeyField";
import "./settings.css";

const COLORS = ["#FF9F0A", "#FFD60A", "#30D158", "#64D2FF", "#0A84FF", "#BF5AF2", "#FF375F"];

const TONES: Tone[] = ["bell", "soft", "drop"];

const FULLSCREEN_OPTIONS: Fullscreen[] = ["show", "alerts", "hide"];

const VIEWER_BAR_OPTIONS: ViewerBar[] = ["strip", "floating"];

const LANGUAGE_OPTIONS: Language[] = ["auto", "ru", "en"];

export function SettingsPage() {
  const { t } = useTranslation();
  const [settings, setSettings] = useState<Settings | null>(null);
  const [status, setStatus] = useState<string>("");
  const [defaultShots, setDefaultShots] = useState<string>("");
  const [defaultRecordings, setDefaultRecordings] = useState<string>("");
  const [displays, setDisplays] = useState<Display[]>([]);
  const [info, setInfo] = useState<AppInfo | null>(null);
  // Where the update stands: the island keeps it, this window only shows it.
  const [updating, setUpdating] = useState<UpdateView | null>(null);

  useEffect(() => {
    void api.getSettings().then(setSettings);
    void api.defaultScreenshotDir().then(setDefaultShots);
    void api.defaultRecordingsDir().then(setDefaultRecordings);
    void api.displays().then(setDisplays);
    void api.appInfo().then(setInfo);
    const poll = () => void api.updateStatus().then(setUpdating);
    poll();
    const id = window.setInterval(poll, 700);
    return () => window.clearInterval(id);
  }, []);

  if (!settings) return null;

  const update = (patch: Partial<Settings>) => {
    setSettings({ ...settings, ...patch });
    setStatus("");
  };
  const updateReminder = (id: string, patch: Partial<Reminder>) =>
    update({ reminders: settings.reminders.map((r) => (r.id === id ? { ...r, ...patch } : r)) });
  const removeReminder = (id: string) => update({ reminders: settings.reminders.filter((r) => r.id !== id) });
  const addReminder = () =>
    update({
      reminders: [
        ...settings.reminders,
        {
          id: crypto.randomUUID(),
          name: t("reminders.new_name"),
          color: COLORS[(settings.reminders.length + 2) % COLORS.length],
          tone: "bell",
          intervalMin: 45,
          durationSec: 30,
          enabled: true,
          resetOnIdle: true,
        },
      ],
    });

  const save = async () => {
    try {
      const saved = await api.saveSettings(settings);
      setSettings(saved);
      setStatus(t((await api.languagePending(saved.language)) ? "footer.saved_restart" : "footer.saved"));
    } catch (e) {
      setStatus(t("footer.save_failed", { error: String(e) }));
    }
  };

  return (
    <main className="settings">
      <h1>{t("reminders.title")}</h1>
      <div className="reminders">
        {settings.reminders.map((r) => (
          <ReminderCard
            key={r.id}
            reminder={r}
            volume={settings.volume}
            onChange={(patch) => updateReminder(r.id, patch)}
            onRemove={() => removeReminder(r.id)}
          />
        ))}
        <button className="add" onClick={addReminder}>
          {t("reminders.add")}
        </button>
      </div>

      <h1>{t("general.title")}</h1>
      <section className="card general">
        <div className="choice">
          <label className="line">
            <span>{t("general.language")}</span>
            <select value={settings.language} onChange={(e) => update({ language: e.target.value as Language })}>
              {LANGUAGE_OPTIONS.map((value) => (
                <option key={value} value={value}>
                  {t(`language.${value}`)}
                </option>
              ))}
            </select>
          </label>
          <span className="hint">{t("general.language_hint")}</span>
        </div>
        <label className="line">
          <span>{t("general.volume")}</span>
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={settings.volume}
            onChange={(e) => update({ volume: Number(e.target.value) })}
          />
        </label>
        <label className="line">
          <span>{t("general.idle_before")}</span>
          <span className="unit">
            <NumberInput value={settings.idleMin} min={1} max={120} onChange={(idleMin) => update({ idleMin })} />
            {t("general.idle_after")}
          </span>
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={settings.showIsland}
            onChange={(e) => update({ showIsland: e.target.checked })}
          />
          {t("general.show_island")}
        </label>
        <label className="line">
          <span>{t("general.monitor")}</span>
          <select value={settings.islandMonitor} onChange={(e) => update({ islandMonitor: e.target.value })}>
            <option value="">{t("general.monitor_follow")}</option>
            {displays.map((d) => (
              <option key={d.name} value={d.name}>
                {t(d.primary ? "general.monitor_main" : "general.monitor_item", {
                  n: d.name.replace(/D+/g, ""),
                  size: `${d.width}×${d.height}`,
                })}
              </option>
            ))}
            {settings.islandMonitor !== "" && !displays.some((d) => d.name === settings.islandMonitor) && (
              <option value={settings.islandMonitor}>{t("general.monitor_gone")}</option>
            )}
          </select>
        </label>
        <fieldset className="choice">
          <legend>{t("general.fullscreen")}</legend>
          <div className="options">
            {FULLSCREEN_OPTIONS.map((value) => (
              <label key={value} className="check">
                <input
                  type="radio"
                  name="fullscreen"
                  checked={settings.fullscreen === value}
                  onChange={() => update({ fullscreen: value })}
                />
                {t(`fullscreen.${value}`)}
              </label>
            ))}
          </div>
          <span className="hint">{t("general.fullscreen_hint")}</span>
        </fieldset>
        <label className="check">
          <input
            type="checkbox"
            checked={settings.player}
            onChange={(e) => update({ player: e.target.checked })}
          />
          {t("general.player")}
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={settings.announceTracks}
            disabled={!settings.player}
            onChange={(e) => update({ announceTracks: e.target.checked })}
          />
          {t("general.announce_tracks")}
        </label>
        <label className="line">
          <span>{t("general.silent_drop")}</span>
          <span className="unit">
            <NumberInput
              value={settings.silentDropMin}
              min={0}
              max={240}
              onChange={(silentDropMin) => update({ silentDropMin })}
            />
            {t("general.silent_drop_unit")}
          </span>
        </label>
        <div className="choice">
          <label className="check">
            <input
              type="checkbox"
              checked={settings.liveWhenDimmed}
              onChange={(e) => update({ liveWhenDimmed: e.target.checked })}
            />
            {t("general.live")}
          </label>
          <span className="hint">{t("general.live_hint")}</span>
        </div>
        <div className="choice">
          <label className="check">
            <input
              type="checkbox"
              checked={settings.keepScreenOn}
              onChange={(e) => update({ keepScreenOn: e.target.checked })}
            />
            {t("general.keep_screen")}
          </label>
          <span className="hint">{t("general.keep_screen_hint")}</span>
        </div>
        <div className="choice">
          <label className="check">
            <input
              type="checkbox"
              checked={settings.animations}
              onChange={(e) => update({ animations: e.target.checked })}
            />
            {t("general.animations")}
          </label>
          <span className="hint">{t("general.animations_hint")}</span>
        </div>
        <div className="choice">
          <label className="check">
            <input
              type="checkbox"
              checked={settings.areaMarks}
              onChange={(e) => update({ areaMarks: e.target.checked })}
            />
            {t("general.area_marks")}
          </label>
          <span className="hint">{t("general.area_marks_hint")}</span>
        </div>
        <HotkeyField
          label={t("shot.key")}
          hint={t("shot.key_hint")}
          value={settings.screenshotKey}
          onChange={(screenshotKey) => update({ screenshotKey })}
          printScreen
          others={[
            { key: settings.areaKey, is: "area" },
            { key: settings.recordKey, is: "record" },
          ]}
        />
        <HotkeyField
          label={t("shot.area_key")}
          hint={t("shot.area_key_hint")}
          value={settings.areaKey}
          onChange={(areaKey) => update({ areaKey })}
          printScreen
          others={[
            { key: settings.screenshotKey, is: "shot" },
            { key: settings.recordKey, is: "record" },
          ]}
        />
        <div className="choice">
          <span>{t("shot.folder")}</span>
          <div className="folder">
            <span className="path" title={settings.screenshotDir || defaultShots}>
              {settings.screenshotDir || t("folder.default_path", { path: defaultShots })}
            </span>
            <button
              onClick={() =>
                void api.pickFolder(settings.screenshotDir).then((dir) => dir && update({ screenshotDir: dir }))
              }
            >
              {t("folder.choose")}
            </button>
            {settings.screenshotDir && (
              <button onClick={() => update({ screenshotDir: "" })}>{t("folder.reset")}</button>
            )}
          </div>
          <span className="hint">{t("shot.hint")}</span>
        </div>
        <fieldset className="choice">
          <legend>{t("viewer.title")}</legend>
          <div className="options">
            {VIEWER_BAR_OPTIONS.map((value) => (
              <label key={value} className="check">
                <input
                  type="radio"
                  name="viewerBar"
                  checked={settings.viewerBar === value}
                  onChange={() => update({ viewerBar: value })}
                />
                {t(`viewer.${value}`)}
              </label>
            ))}
          </div>
          <span className="hint">{t("viewer.hint")}</span>
        </fieldset>
        <HotkeyField
          label={t("record.key")}
          hint={t("record.key_hint")}
          value={settings.recordKey}
          onChange={(recordKey) => update({ recordKey })}
          printScreen={false}
          others={[
            { key: settings.screenshotKey, is: "shot" },
            { key: settings.areaKey, is: "area" },
          ]}
        />
        <span className="hint">{t("record.panel_hint")}</span>
        <div className="choice">
          <span>{t("record.folder")}</span>
          <div className="folder">
            <span className="path" title={settings.recordingsDir || defaultRecordings}>
              {settings.recordingsDir || t("folder.default_path", { path: defaultRecordings })}
            </span>
            <button
              onClick={() =>
                void api
                  .pickFolder(settings.recordingsDir, true)
                  .then((dir) => dir && update({ recordingsDir: dir }))
              }
            >
              {t("folder.choose")}
            </button>
            {settings.recordingsDir && (
              <button onClick={() => update({ recordingsDir: "" })}>{t("folder.reset")}</button>
            )}
          </div>
          <span className="hint">{t("record.hint")}</span>
        </div>
        <label className="check">
          <input
            type="checkbox"
            checked={settings.autostart}
            onChange={(e) => update({ autostart: e.target.checked })}
          />
          {t("general.autostart")}
        </label>
      </section>

      <h1>{t("dev.title")}</h1>
      <section className="card general">
        <div className="choice">
          <span>{t("dev.journal")}</span>
          <div className="folder">
            <button
              onClick={() =>
                void api.openJournal().then((opened) => setStatus(opened ? "" : t("dev.journal_failed")))
              }
            >
              {t("dev.open_journal")}
            </button>
          </div>
          <span className="hint">{t("dev.journal_hint")}</span>
        </div>
      </section>

      <h1>{t("about.title")}</h1>
      <section className="card general">
        {info && <span>{t("about.version", { version: info.version, license: info.license })}</span>}
        <div className="folder">
          <button
            disabled={updating?.status.phase === "checking" || updating?.running === false}
            onClick={() => void api.updateCommand("check")}
          >
            {t("about.check")}
          </button>
          {updating?.status.phase === "available" && updating.status.offer && (
            <button className="primary" onClick={() => void api.updateCommand("openWindow")}>
              {t("about.install", { version: updating.status.offer.latest })}
            </button>
          )}
          <span className="hint">{updateText(updating, t)}</span>
        </div>
        <label className="check">
          <input
            type="checkbox"
            checked={settings.checkUpdates}
            onChange={(e) => update({ checkUpdates: e.target.checked })}
          />
          {t("about.auto")}
        </label>
        <div className="folder">
          <button onClick={() => void api.openProjectPage("home")}>{t("about.home")}</button>
          <button onClick={() => void api.openProjectPage("changes")}>{t("about.changes")}</button>
          <button onClick={() => void api.openProjectPage("license")}>{t("about.license")}</button>
        </div>
      </section>

      <footer>
        <span className="status">{status}</span>
        <button onClick={() => void getCurrentWindow().close()}>{t("footer.close")}</button>
        <button className="primary" onClick={() => void save()}>
          {t("footer.save")}
        </button>
      </footer>
    </main>
  );
}

interface ReminderCardProps {
  reminder: Reminder;
  volume: number;
  onChange: (patch: Partial<Reminder>) => void;
  onRemove: () => void;
}

type Translate = ReturnType<typeof useTranslation>["t"];

/** The line next to "Check for updates": what the island says about the update now. */
function updateText(view: UpdateView | null, t: Translate): string {
  if (!view) return "";
  if (!view.running) return t("about.no_island");
  const { status } = view;
  switch (status.phase) {
    case "checking":
      return t("about.checking");
    case "upToDate":
      return t("about.up_to_date");
    case "available":
      return status.offer
        ? t(status.offer.skipped ? "about.available_skipped" : "about.available", { version: status.offer.latest })
        : "";
    case "failed":
    case "installFailed":
      return t("about.failed", { error: status.error ?? "" });
    case "downloading":
    case "verifying":
    case "installing":
      return t("about.downloading");
    default:
      return "";
  }
}

function ReminderCard({ reminder: r, volume, onChange, onRemove }: ReminderCardProps) {
  const { t } = useTranslation();
  return (
    <section className={`card reminder ${r.enabled ? "" : "disabled"}`} style={{ borderLeftColor: r.color }}>
      <div className="head">
        <input
          type="checkbox"
          title={t("reminder.enabled")}
          checked={r.enabled}
          onChange={(e) => onChange({ enabled: e.target.checked })}
        />
        <input className="name" value={r.name} maxLength={40} onChange={(e) => onChange({ name: e.target.value })} />
        <div className="colors">
          {COLORS.map((c) => (
            <button
              key={c}
              className={`swatch ${c === r.color ? "selected" : ""}`}
              style={{ background: c }}
              title={c}
              onClick={() => onChange({ color: c })}
            />
          ))}
        </div>
        <button className="icon" title={t("reminder.remove")} onClick={onRemove}>
          ✕
        </button>
      </div>
      <div className="fields">
        <label className="unit">
          {t("reminder.every")}
          <NumberInput value={r.intervalMin} min={1} max={600} onChange={(intervalMin) => onChange({ intervalMin })} />
          {t("reminder.min")}
        </label>
        <label className="unit">
          {t("reminder.show_for")}
          <NumberInput value={r.durationSec} min={5} max={300} onChange={(durationSec) => onChange({ durationSec })} />
          {t("reminder.sec")}
        </label>
        <span className="unit">
          <select value={r.tone} onChange={(e) => onChange({ tone: e.target.value as Tone })}>
            {TONES.map((tone) => (
              <option key={tone} value={tone}>
                {t(`tone.${tone}`)}
              </option>
            ))}
          </select>
          <button className="icon" title={t("reminder.preview")} onClick={() => void api.previewSound(r.tone, volume)}>
            ▶
          </button>
        </span>
      </div>
      <label className="check small">
        <input
          type="checkbox"
          checked={r.resetOnIdle}
          onChange={(e) => onChange({ resetOnIdle: e.target.checked })}
        />
        {t("reminder.reset_on_idle")}
      </label>
    </section>
  );
}

interface NumberInputProps {
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
}

function NumberInput({ value, min, max, onChange }: NumberInputProps) {
  const [text, setText] = useState(String(value));
  useEffect(() => setText(String(value)), [value]);

  const commit = () => {
    const parsed = Math.round(Number(text));
    const next = Number.isFinite(parsed) && text.trim() !== "" ? Math.min(max, Math.max(min, parsed)) : value;
    setText(String(next));
    if (next !== value) onChange(next);
  };

  return (
    <input
      className="number"
      inputMode="numeric"
      value={text}
      onChange={(e) => {
        setText(e.target.value);
        const parsed = Number(e.target.value);
        if (e.target.value.trim() !== "" && Number.isInteger(parsed) && parsed >= min && parsed <= max) {
          onChange(parsed);
        }
      }}
      onBlur={commit}
    />
  );
}
