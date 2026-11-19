import { useTranslation } from "react-i18next";
import { api, type Fullscreen, type ViewerBar } from "../../api";
import { HotkeyField } from "../HotkeyField";
import { useNav } from "../Navigator";
import { useSettings } from "../state";
import { Group, Mini, NumberRow, RadioRow, Row, Segmented, ToggleRow } from "../ui";

export function IslandPage() {
  const { t } = useTranslation();
  const { go } = useNav();
  const { settings, update, info } = useSettings();
  const fullscreen: Fullscreen[] = ["show", "alerts", "hide"];
  const viewer: ViewerBar[] = ["strip", "floating"];

  return (
    <>
      <Group plain>
        <ToggleRow
          label={t("island.show")}
          sub={t("island.show_sub")}
          on={settings.showIsland}
          onChange={(showIsland) => update({ showIsland })}
        />
        <Row label={t("island.monitor")} value={useMonitorName()} opens onClick={() => go({ id: "monitor" })} />
      </Group>
      <Group title={t("island.control_center")} plain>
        <ToggleRow
          label={t("island.seconds")}
          on={settings.clockSeconds}
          onChange={(clockSeconds) => update({ clockSeconds })}
        />
        {info?.battery && (
          <ToggleRow
            label={t("island.battery_percent")}
            on={settings.batteryPercent}
            onChange={(batteryPercent) => update({ batteryPercent })}
          />
        )}
      </Group>
      <Group title={t("island.fullscreen")} plain>
        <div className="row block noicon">
          <Segmented
            value={settings.fullscreen}
            options={fullscreen.map((value) => ({ value, label: t(`fullscreen.${value}`) }))}
            onChange={(value) => update({ fullscreen: value })}
          />
        </div>
      </Group>
      <Group title={t("viewer.title")} plain>
        <div className="row block noicon">
          <Segmented
            value={settings.viewerBar}
            options={viewer.map((value) => ({ value, label: t(`viewer.${value}`) }))}
            onChange={(value) => update({ viewerBar: value })}
          />
        </div>
      </Group>
    </>
  );
}

function useMonitorName(): string {
  const { t } = useTranslation();
  const { settings, displays } = useSettings();
  if (settings.islandMonitor === "") return t("monitor.follow");
  const display = displays.find((d) => d.name === settings.islandMonitor);
  return display ? monitorLabel(display, t) : t("monitor.gone");
}

function monitorLabel(
  display: { name: string; primary: boolean; width: number; height: number },
  t: ReturnType<typeof useTranslation>["t"],
): string {
  return t(display.primary ? "monitor.main" : "monitor.item", {
    n: display.name.replace(/\D+/g, ""),
    size: `${display.width}×${display.height}`,
  });
}

export function MonitorPage() {
  const { t } = useTranslation();
  const { settings, update, displays } = useSettings();
  const gone = settings.islandMonitor !== "" && !displays.some((d) => d.name === settings.islandMonitor);

  return (
    <Group plain>
      <RadioRow
        label={t("monitor.follow")}
        on={settings.islandMonitor === ""}
        onClick={() => update({ islandMonitor: "" })}
      />
      {displays.map((d) => (
        <RadioRow
          key={d.name}
          label={monitorLabel(d, t)}
          on={settings.islandMonitor === d.name}
          onClick={() => update({ islandMonitor: d.name })}
        />
      ))}
      {gone && <RadioRow label={t("monitor.gone")} on onClick={() => {}} />}
    </Group>
  );
}

export function PlayerPage() {
  const { t } = useTranslation();
  const { settings, update } = useSettings();
  return (
    <Group footer={t("player.silent_drop_hint")} plain>
      <ToggleRow label={t("player.show")} on={settings.player} onChange={(player) => update({ player })} />
      <ToggleRow
        label={t("player.announce")}
        on={settings.announceTracks}
        disabled={!settings.player}
        onChange={(announceTracks) => update({ announceTracks })}
      />
      <NumberRow
        label={t("player.silent_drop")}
        unit={t("unit.min")}
        value={settings.silentDropMin}
        min={0}
        max={240}
        onChange={(silentDropMin) => update({ silentDropMin })}
      />
    </Group>
  );
}

/** Without a battery the charge rows are left out. */
export function NotificationsPage() {
  const { t } = useTranslation();
  const { settings, update, info } = useSettings();
  const battery = info?.battery ?? false;
  return (
    <Group plain>
      {battery && (
        <ToggleRow
          label={t("notify.low")}
          sub={t("notify.low_sub")}
          on={settings.notifyLowCharge}
          onChange={(notifyLowCharge) => update({ notifyLowCharge })}
        />
      )}
      {battery && (
        <ToggleRow
          label={t("notify.charging")}
          sub={t("notify.charging_sub")}
          on={settings.notifyCharging}
          onChange={(notifyCharging) => update({ notifyCharging })}
        />
      )}
      <ToggleRow
        label={t(battery ? "notify.usage" : "notify.load")}
        sub={t(battery ? "notify.usage_sub" : "notify.load_sub")}
        on={settings.notifyUsage}
        onChange={(notifyUsage) => update({ notifyUsage })}
      />
      <ToggleRow
        label={t("notify.downloads")}
        sub={t("notify.downloads_sub")}
        on={settings.notifyDownloads}
        onChange={(notifyDownloads) => update({ notifyDownloads })}
      />
      <ToggleRow
        label={t("notify.devices")}
        sub={t("notify.devices_sub")}
        on={settings.notifyDevices}
        onChange={(notifyDevices) => update({ notifyDevices })}
      />
      <ToggleRow
        label={t("notify.device_battery")}
        sub={t("notify.device_battery_sub")}
        on={settings.notifyDeviceBattery}
        onChange={(notifyDeviceBattery) => update({ notifyDeviceBattery })}
      />
      <ToggleRow
        label={t("notify.audio_switch")}
        sub={t("notify.audio_switch_sub")}
        on={settings.notifyAudioSwitch}
        onChange={(notifyAudioSwitch) => update({ notifyAudioSwitch })}
      />
    </Group>
  );
}

export function ShotsPage() {
  const { t } = useTranslation();
  const { settings, update, defaultShots } = useSettings();
  return (
    <>
      <Group title={t("shots.keys")} plain>
        <HotkeyField
          label={t("shots.key")}
          value={settings.screenshotKey}
          onChange={(screenshotKey) => update({ screenshotKey })}
          printScreen
          others={[
            { key: settings.areaKey, is: "area" },
            { key: settings.recordKey, is: "record" },
          ]}
        />
        <HotkeyField
          label={t("shots.area_key")}
          value={settings.areaKey}
          onChange={(areaKey) => update({ areaKey })}
          printScreen
          others={[
            { key: settings.screenshotKey, is: "shot" },
            { key: settings.recordKey, is: "record" },
          ]}
        />
      </Group>
      <Group plain>
        <FolderRow
          path={settings.screenshotDir}
          fallback={defaultShots}
          onChange={(screenshotDir) => update({ screenshotDir })}
        />
      </Group>
    </>
  );
}

export function RecordPage() {
  const { t } = useTranslation();
  const { settings, update, defaultRecordings } = useSettings();
  return (
    <>
      <Group title={t("record.key_group")} plain>
        <HotkeyField
          label={t("record.key")}
          value={settings.recordKey}
          onChange={(recordKey) => update({ recordKey })}
          printScreen={false}
          others={[
            { key: settings.screenshotKey, is: "shot" },
            { key: settings.areaKey, is: "area" },
          ]}
        />
      </Group>
      <Group plain>
        <FolderRow
          recordings
          path={settings.recordingsDir}
          fallback={defaultRecordings}
          onChange={(recordingsDir) => update({ recordingsDir })}
        />
        <ToggleRow
          label={t("record.marks")}
          on={settings.areaMarks}
          onChange={(areaMarks) => update({ areaMarks })}
        />
      </Group>
    </>
  );
}

/** `path` empty = the default folder, `fallback`. */
function FolderRow({
  path,
  fallback,
  recordings = false,
  onChange,
}: {
  path: string;
  fallback: string;
  recordings?: boolean;
  onChange: (path: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <Row
      label={t("folder.title")}
      sub={<span title={path || fallback}>{path || t("folder.default_path", { path: fallback })}</span>}
      className="folder"
    >
      <Mini onClick={() => void api.pickFolder(path, recordings).then((dir) => dir && onChange(dir))}>
        {t("folder.choose")}
      </Mini>
      {path && <Mini onClick={() => onChange("")}>{t("folder.reset")}</Mini>}
    </Row>
  );
}
