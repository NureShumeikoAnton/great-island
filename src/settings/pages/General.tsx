import { useTranslation } from "react-i18next";
import { api, type Language } from "../../api";
import { AppIcon } from "../../AppIcon";
import { updateLine, useSettings } from "../state";
import { Group, RadioRow, Row, ToggleRow } from "../ui";
import { CheckRow } from "./Main";

export function EnergyPage() {
  const { t } = useTranslation();
  const { settings, update } = useSettings();
  return (
    <>
      <Group plain>
        <ToggleRow
          label={t("energy.animations")}
          on={settings.animations}
          onChange={(animations) => update({ animations })}
        />
        <ToggleRow
          label={t("energy.live")}
          sub={t("energy.live_sub")}
          on={settings.liveWhenDimmed}
          onChange={(liveWhenDimmed) => update({ liveWhenDimmed })}
        />
      </Group>
      <Group title={t("energy.keep_awake")} plain>
        <ToggleRow
          label={t("energy.keep_screen")}
          on={settings.keepScreenOn}
          onChange={(keepScreenOn) => update({ keepScreenOn })}
        />
      </Group>
    </>
  );
}

export function LanguagePage() {
  const { t } = useTranslation();
  const { settings, update } = useSettings();
  const options: Language[] = ["auto", "ru", "en"];
  return (
    <Group footer={t("language.hint")} plain>
      {options.map((language) => (
        <RadioRow
          key={language}
          label={t(`language.${language}`)}
          on={settings.language === language}
          onClick={() => update({ language })}
        />
      ))}
    </Group>
  );
}

export function AboutPage() {
  const { t, i18n } = useTranslation();
  const { settings, update, info, updating } = useSettings();
  const status = updating?.status;
  const failed = status && (status.phase === "failed" || status.phase === "installFailed") ? status.error : null;

  const open = (page: "home" | "changes" | "license") => () => void api.openProjectPage(page);
  return (
    <>
      <div className="about">
        <AppIcon className="about-icon" />
        <div className="name">Great Island</div>
        {info && <div className="sub">{t("about.version_license", { version: info.version, license: info.license })}</div>}
        <div className="sub">{updateLine(updating, t, i18n.language)}</div>
        {failed && <div className="sub bad">{failed}</div>}
      </div>
      <Group plain>
        <CheckRow />
        <ToggleRow
          label={t("about.auto")}
          on={settings.checkUpdates}
          onChange={(checkUpdates) => update({ checkUpdates })}
        />
      </Group>
      <Group plain>
        <Row label={t("about.home")} external onClick={open("home")} />
        <Row label={t("about.changes")} external onClick={open("changes")} />
        <Row label={t("about.license")} external onClick={open("license")} />
      </Group>
    </>
  );
}
