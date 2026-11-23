import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { AppIcon } from "../../AppIcon";
import { Glyph } from "../icons";
import { useNav } from "../Navigator";
import { updateLine, useKeyLabel, useSettings } from "../state";
import { Group, Row, ToggleRow } from "../ui";

export function MainPage() {
  const { t, i18n } = useTranslation();
  const { go } = useNav();
  const { settings, update, info, updating, problemsToday, apps, appOn } = useSettings();
  const shotKey = useKeyLabel(settings.screenshotKey);
  const recordKey = useKeyLabel(settings.recordKey);
  const on = settings.reminders.filter((r) => r.enabled).length;
  const battery = info?.battery ?? false;
  const notifying =
    settings.notifyUsage ||
    settings.notifyDownloads ||
    settings.notifyDevices ||
    settings.notifyDeviceBattery ||
    settings.notifyAudioSwitch ||
    (battery && (settings.notifyLowCharge || settings.notifyCharging));

  return (
    <>
      <section className="group">
        <div className="card">
          <div className="hero" role="button" onClick={() => go({ id: "about" })}>
            <AppIcon className="hero-icon" />
            <div className="hero-text">
              <div className="name">Great Island</div>
              <div className="sub">{info && t("about.version", { version: info.version })}</div>
              <div className="sub">{updateLine(updating, t, i18n.language)}</div>
            </div>
            <span className="chev">
              <Glyph name="chevron" size={16} />
            </span>
          </div>
        </div>
      </section>

      <Group title={t("main.features")}>
        <Row
          icon={{ glyph: "bell", color: "var(--c-orange)" }}
          label={t("page.reminders")}
          value={t("main.reminders_value", { on, all: settings.reminders.length })}
          opens
          onClick={() => go({ id: "reminders" })}
        />
        <Row
          icon={{ glyph: "island", color: "var(--c-indigo)" }}
          label={t("page.island")}
          value={t(settings.showIsland ? "island.always" : "island.with_reminders")}
          opens
          onClick={() => go({ id: "island" })}
        />
        <Row
          icon={{ glyph: "music", color: "var(--c-pink)" }}
          label={t("page.player")}
          value={t(settings.player ? "main.on" : "main.off")}
          opens
          onClick={() => go({ id: "player" })}
        />
        <Row
          icon={{ glyph: "camera", color: "var(--c-blue)" }}
          label={t("page.shots")}
          value={shotKey}
          opens
          onClick={() => go({ id: "shots" })}
        />
        <Row
          icon={{ glyph: "record", color: "var(--c-red)" }}
          label={t("page.record")}
          value={recordKey}
          opens
          onClick={() => go({ id: "record" })}
        />
        <Row
          icon={{ glyph: "battery", color: "var(--c-green)" }}
          label={t("page.notifications")}
          value={t(notifying ? "main.on" : "main.off")}
          opens
          onClick={() => go({ id: "notifications" })}
        />
        <Row
          icon={{ glyph: "apps", color: "var(--c-purple)" }}
          label={t("page.apps")}
          value={t("main.apps_value", { on: apps.filter(appOn).length, all: apps.length })}
          opens
          onClick={() => go({ id: "apps" })}
        />
      </Group>

      <Group title={t("main.general")}>
        <Row
          icon={{ glyph: "zap", color: "var(--c-green)" }}
          label={t("page.energy")}
          opens
          onClick={() => go({ id: "energy" })}
        />
        <Row
          icon={{ glyph: "language", color: "var(--c-teal)" }}
          label={t("page.language")}
          value={t(settings.language === "auto" ? "language.auto_short" : `language.${settings.language}`)}
          opens
          onClick={() => go({ id: "language" })}
        />
        <ToggleRow
          icon={{ glyph: "power", color: "var(--c-gray)" }}
          label={t("main.autostart")}
          on={settings.autostart}
          onChange={(autostart) => update({ autostart })}
        />
      </Group>

      <Group title={t("main.service")}>
        <CheckRow icon />
        <Row
          icon={{ glyph: "journal", color: "var(--c-brown)" }}
          label={t("page.journal")}
          value={problemsToday > 0 ? <span className="bad">{t("main.problems_today", { count: problemsToday })}</span> : undefined}
          opens
          onClick={() => go({ id: "journal" })}
        />
      </Group>
    </>
  );
}

/** "Check for updates", with the answer in the row; with a release waiting it opens the update window. */
export function CheckRow({ icon }: { icon?: boolean }) {
  const { t } = useTranslation();
  const { updating } = useSettings();
  const phase = updating?.status.phase;
  const offer = updating?.status.offer ?? null;
  const running = updating?.running ?? false;
  const available = phase === "available" && offer !== null;
  const busy = phase === "checking" || phase === "downloading" || phase === "verifying" || phase === "installing";

  let value = null;
  if (!running) value = t("about.no_island");
  else if (phase === "checking")
    value = (
      <>
        <span className="spin" />
        {t("about.checking")}
      </>
    );
  else if (available) value = <span className="accent">{t("about.available", { version: offer.latest })}</span>;
  else if (phase === "upToDate" && updating?.status.manual) value = <span className="good">{t("about.latest")}</span>;
  else if (phase === "failed") value = <span className="bad">{t("about.failed_short")}</span>;

  return (
    <Row
      icon={icon ? { glyph: "refresh", color: "var(--c-blue)" } : undefined}
      label={t("main.check")}
      value={value ?? undefined}
      className={!running || busy ? "disabled" : ""}
      onClick={running && !busy ? () => void api.updateCommand(available ? "openWindow" : "check") : undefined}
    />
  );
}
