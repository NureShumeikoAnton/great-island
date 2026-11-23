// Apps & Connectors: a switch per app, a page for an app with settings of its own, the terms a
// connector shows before it is turned on, and a lookup page for a long list (a place). The list,
// the titles and the fields come from the island's catalog (`list_apps`), so a new app needs
// nothing here but its icon.

import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { api, type AppView, type Choice, type FieldView } from "../../api";
import { Glyph, Tile, type GlyphName } from "../icons";
import { useNav } from "../Navigator";
import { useSettings } from "../state";
import { Group, Row, Switch, ToggleRow } from "../ui";

/** The island's tile has the same glyph and color (`island/view/app.rs`). */
const ICONS: Record<AppView["icon"], { glyph: GlyphName; color: string }> = {
  calculator: { glyph: "calculator", color: "var(--c-orange)" },
  siren: { glyph: "siren", color: "var(--c-red)" },
};

const hasSettings = (app: AppView) => app.connector !== null || app.fields.length > 0;

/** Turning a connector on asks for its terms first; turning it off doesn't. */
function useSwitch(): {
  terms: ReactNode;
  set: (app: AppView, on: boolean) => void;
} {
  const { setAppOn, appValue } = useSettings();
  const { go } = useNav();
  const [asking, setAsking] = useState<AppView | null>(null);
  const set = (app: AppView, on: boolean) => {
    if (on && app.connector) setAsking(app);
    else setAppOn(app.id, on);
  };
  const terms = asking && (
    <TermsDialog
      app={asking}
      onCancel={() => setAsking(null)}
      onConfirm={() => {
        setAppOn(asking.id, true);
        setAsking(null);
        // A connector that has nothing to watch yet goes to where it is chosen.
        const lookup = asking.fields.find((f) => f.kind === "lookup");
        if (lookup && !appValue(asking, lookup.key)) go({ id: "lookup", param: `${asking.id}/${lookup.key}` });
      }}
    />
  );
  return { terms, set };
}

export function AppsPage() {
  const { t } = useTranslation();
  const { go } = useNav();
  const { apps, appOn } = useSettings();
  const { terms, set } = useSwitch();
  const row = (app: AppView) =>
    hasSettings(app) ? (
      <Row key={app.id} icon={ICONS[app.icon]} label={app.title} opens onClick={() => go({ id: "app", param: app.id })}>
        <Switch on={appOn(app)} onChange={(on) => set(app, on)} />
      </Row>
    ) : (
      <ToggleRow key={app.id} icon={ICONS[app.icon]} label={app.title} on={appOn(app)} onChange={(on) => set(app, on)} />
    );
  const plain = apps.filter((app) => !app.connector);
  const connectors = apps.filter((app) => app.connector);
  return (
    <>
      <Group title={t("apps.apps")} footer={t("apps.apps_footer")}>
        {plain.map(row)}
      </Group>
      {connectors.length > 0 && (
        <Group title={t("apps.connectors")} footer={t("apps.connectors_footer")}>
          {connectors.map(row)}
        </Group>
      )}
      {terms}
    </>
  );
}

export function AppPage({ id }: { id: string }) {
  const { t } = useTranslation();
  const { go } = useNav();
  const { apps, appOn } = useSettings();
  const { terms, set } = useSwitch();
  const app = apps.find((a) => a.id === id);
  if (!app) return null;
  // Fields are grouped where a field starts a group; the first group has the switch.
  const groups: { title: string; fields: FieldView[] }[] = [];
  for (const field of app.fields) {
    if (field.group !== null || groups.length === 0) groups.push({ title: field.group ?? "", fields: [] });
    groups[groups.length - 1].fields.push(field);
  }
  return (
    <>
      {app.connector && <ServiceHead app={app} />}
      <Group footer={app.connector ? undefined : t("apps.app_footer")} plain>
        <ToggleRow label={app.title} on={appOn(app)} onChange={(on) => set(app, on)} />
        {app.connector && <Row label={t("apps.terms")} opens onClick={() => go({ id: "terms", param: app.id })} />}
      </Group>
      {groups.map((group, i) => (
        <Group key={i} title={group.title || undefined} plain>
          {group.fields.map((field) => (
            <FieldRow key={field.key} app={app} field={field} />
          ))}
        </Group>
      ))}
      {terms}
    </>
  );
}

function ServiceHead({ app }: { app: AppView }) {
  const service = app.connector!;
  return (
    <div className="service-head">
      <Tile glyph={ICONS[app.icon].glyph} color={ICONS[app.icon].color} />
      <span className="service-text">
        <span className="service-name">{app.title}</span>
        <button className="service-link" onClick={() => void api.openAppSite(app.id)}>
          {service.provider} · {new URL(service.site).host}
        </button>
      </span>
    </div>
  );
}

/** What is said before the switch turns on, and on its own page afterwards. */
function TermsText({ app }: { app: AppView }) {
  const { t } = useTranslation();
  const service = app.connector!;
  return (
    <div className="terms">
      <p>{t("apps.terms_intro", { provider: service.provider })}</p>
      <h3>{t("apps.terms_sends")}</h3>
      <ol>
        {service.sends.map((line, i) => (
          <li key={i}>{line}</li>
        ))}
      </ol>
      {service.note && <p className="muted">{service.note}</p>}
    </div>
  );
}

function TermsDialog({ app, onCancel, onConfirm }: { app: AppView; onCancel: () => void; onConfirm: () => void }) {
  const { t } = useTranslation();
  useEffect(() => {
    // Esc closes the dialog, not the page under it.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopImmediatePropagation();
      onCancel();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onCancel]);
  return (
    <div className="dim" onClick={onCancel}>
      <div className="dialog" role="dialog" aria-modal="true" aria-label={t("apps.terms")} onClick={(e) => e.stopPropagation()}>
        <ServiceHead app={app} />
        <div className="dialog-body">
          <TermsText app={app} />
        </div>
        <div className="dialog-foot">
          <button className="btn" onClick={onCancel}>
            {t("apps.cancel")}
          </button>
          <button className="btn primary" onClick={onConfirm}>
            {t("apps.confirm")}
          </button>
        </div>
      </div>
    </div>
  );
}

export function TermsPage({ id }: { id: string }) {
  const { apps } = useSettings();
  const app = apps.find((a) => a.id === id);
  if (!app?.connector) return null;
  return (
    <>
      <ServiceHead app={app} />
      <section className="group">
        <div className="card">
          <TermsText app={app} />
        </div>
      </section>
    </>
  );
}

function FieldRow({ app, field }: { app: AppView; field: FieldView }) {
  const { t } = useTranslation();
  const { go } = useNav();
  const { appValue, setAppValue } = useSettings();
  const value = appValue(app, field.key);
  const [shown, setShown] = useState<Choice | null>(null);
  useEffect(() => {
    if (field.kind !== "lookup" || typeof value !== "string" || !value) return setShown(null);
    let live = true;
    void api.appDescribe(app.id, field.key, value).then((c) => live && setShown(c));
    return () => {
      live = false;
    };
  }, [app.id, field.key, field.kind, value]);
  switch (field.kind) {
    case "toggle":
      return (
        <ToggleRow
          label={field.label}
          sub={field.sub || undefined}
          on={value === true}
          onChange={(on) => setAppValue(app.id, field.key, on)}
        />
      );
    case "lookup":
      return (
        <Row
          label={field.label}
          sub={field.sub || undefined}
          value={shown ? shown.label : <span className="accent">{t("apps.not_chosen")}</span>}
          opens
          onClick={() => go({ id: "lookup", param: `${app.id}/${field.key}` })}
        />
      );
    // Drawn with the first app that declares one.
    default:
      return null;
  }
}

/** `param` is "app/key". */
export function LookupPage({ param }: { param: string }) {
  const { t } = useTranslation();
  const { back } = useNav();
  const { apps, appValue, setAppValue } = useSettings();
  const [id, key] = param.split("/");
  const app = apps.find((a) => a.id === id);
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<Choice[]>([]);
  useEffect(() => {
    let live = true;
    void api.appLookup(id, key, query).then((list) => live && setFound(list));
    return () => {
      live = false;
    };
  }, [id, key, query]);
  if (!app) return null;
  const chosen = appValue(app, key);
  return (
    <>
      <label className="search">
        <Glyph name="search" size={15} />
        <input
          autoFocus
          value={query}
          placeholder={t("apps.search")}
          aria-label={t("apps.search")}
          onChange={(e) => setQuery(e.target.value)}
        />
      </label>
      <Group plain>
        {found.length === 0 && <Row label={<span className="muted">{t("apps.nothing_found")}</span>} />}
        {found.map((choice) => (
          <Row
            key={choice.value}
            label={choice.label}
            sub={choice.detail || undefined}
            onClick={() => {
              setAppValue(id, key, choice.value);
              back();
            }}
          >
            <span className="radio">{choice.value === chosen && <Glyph name="check" />}</span>
          </Row>
        ))}
      </Group>
    </>
  );
}
