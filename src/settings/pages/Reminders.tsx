import { useState } from "react";
import { useTranslation } from "react-i18next";
import { api, TONES, type Reminder } from "../../api";
import { Glyph } from "../icons";
import { useNav } from "../Navigator";
import { useSettings } from "../state";
import { Group, NumberInput, NumberRow, RadioRow, Row, Segmented, Switch, ToggleRow } from "../ui";

export const COLORS = ["#FF9F0A", "#FFD60A", "#30D158", "#64D2FF", "#0A84FF", "#BF5AF2", "#FF375F"];

export function RemindersPage() {
  const { t } = useTranslation();
  const { go } = useNav();
  const { settings, update, updateReminder, removeReminder, addReminder } = useSettings();
  // Rows on their way out: they slide away before the list closes over them.
  const [leaving, setLeaving] = useState<string[]>([]);

  const remove = (id: string) => {
    setLeaving((ids) => [...ids, id]);
    window.setTimeout(() => {
      removeReminder(id);
      setLeaving((ids) => ids.filter((x) => x !== id));
    }, 200);
  };

  return (
    <>
      <Group>
        {settings.reminders.map((r) => (
          <Row
            key={r.id}
            className={leaving.includes(r.id) ? "gone" : ""}
            icon={{ glyph: "bell", color: r.color }}
            label={r.name}
            sub={t("reminder.summary", {
              interval: r.intervalMin,
              duration: r.durationSec,
              tone: t(`tone.${r.tone}`),
            })}
            onClick={() => go({ id: "reminder", param: r.id })}
          >
            <Switch on={r.enabled} onChange={(enabled) => updateReminder(r.id, { enabled })} />
            <button
              className="delete"
              title={t("reminder.remove")}
              onClick={(e) => {
                e.stopPropagation();
                remove(r.id);
              }}
            >
              <Glyph name="trash" size={17} />
            </button>
          </Row>
        ))}
        <Row
          className="accent"
          icon={{ glyph: "plus", color: "var(--accent)" }}
          label={t("reminders.add")}
          onClick={() => go({ id: "reminder", param: addReminder() })}
        />
      </Group>

      <Group title={t("reminders.all")} footer={t("reminders.idle_hint")} plain>
        <Row label={t("reminders.volume")}>
          <span className="volume">
            <Glyph name="volumeLow" size={16} />
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={settings.volume}
              onChange={(e) => update({ volume: Number(e.target.value) })}
            />
            <Glyph name="volumeHigh" size={16} />
          </span>
        </Row>
        <NumberRow
          label={t("reminders.idle")}
          unit={t("unit.min")}
          value={settings.idleMin}
          min={1}
          max={120}
          onChange={(idleMin) => update({ idleMin })}
        />
      </Group>
    </>
  );
}

export function ReminderPage({ id }: { id: string }) {
  const { t } = useTranslation();
  const { go, back } = useNav();
  const { settings, updateReminder, removeReminder } = useSettings();
  const r = settings.reminders.find((x) => x.id === id);
  if (!r) return null;
  const change = (patch: Partial<Reminder>) => updateReminder(r.id, patch);

  return (
    <>
      <Group plain>
        <Row label={<input className="name" value={r.name} maxLength={40} onChange={(e) => change({ name: e.target.value })} />} />
        <Row label={<Swatches color={r.color} onPick={(color) => change({ color })} />} />
      </Group>

      <Group plain>
        <Row label={t("reminder.every")}>
          <span className="unit">
            <NumberInput value={r.intervalMin} min={1} max={600} onChange={(intervalMin) => change({ intervalMin })} />
            {t("unit.min")}
          </span>
        </Row>
        <Row label={t("reminder.show_for")}>
          <span className="unit">
            <NumberInput value={r.durationSec} min={5} max={300} onChange={(durationSec) => change({ durationSec })} />
            {t("unit.sec")}
          </span>
        </Row>
        <Row
          label={t("reminder.sound")}
          value={t(`tone.${r.tone}`)}
          opens
          onClick={() => go({ id: "sound", param: r.id })}
        />
      </Group>

      <Group plain>
        <ToggleRow
          label={t("reminder.reset_on_idle")}
          sub={t("reminder.reset_on_idle_sub")}
          on={r.resetOnIdle}
          onChange={(resetOnIdle) => change({ resetOnIdle })}
        />
      </Group>

      <Group plain>
        <Row
          className="danger"
          label={t("reminder.delete")}
          onClick={() => {
            back();
            // After the page has slid away: it must not render a reminder that is gone.
            window.setTimeout(() => removeReminder(r.id), 300);
          }}
        />
      </Group>
    </>
  );
}

function Swatches({ color, onPick }: { color: string; onPick: (color: string) => void }) {
  return (
    <span className="swatches">
      {COLORS.map((c) => (
        <button
          key={c}
          className={c === color ? "on" : ""}
          style={{ ["--c" as string]: c }}
          title={c}
          onClick={() => onPick(c)}
        />
      ))}
    </span>
  );
}

export function SoundPage({ id }: { id: string }) {
  const { t } = useTranslation();
  const { settings, updateReminder } = useSettings();
  const [listen, setListen] = useState<"start" | "end">("start");
  const r = settings.reminders.find((x) => x.id === id);
  if (!r) return null;

  return (
    <>
      <section className="group">
        <Segmented
          value={listen}
          options={[
            { value: "start", label: t("sound.start") },
            { value: "end", label: t("sound.end") },
          ]}
          onChange={setListen}
        />
      </section>
      <Group plain>
        {TONES.map((tone) => (
          <RadioRow
            key={tone}
            label={t(`tone.${tone}`)}
            on={r.tone === tone}
            onClick={() => {
              updateReminder(r.id, { tone });
              void api.previewSound(tone, settings.volume, listen === "end");
            }}
          />
        ))}
      </Group>
    </>
  );
}
