// Texts of the settings window. The island's own texts live in src-tauri/locales.

import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en.json";
import ru from "./locales/ru.json";

// A key missing in either language is a compile error, not a blank label on screen.
type Complete<Other> = Record<keyof Other, string>;
const resources = {
  en: { translation: en satisfies Complete<typeof ru> },
  ru: { translation: ru satisfies Complete<typeof en> },
};

declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: "translation";
    resources: { translation: typeof en };
    // The keys are flat ("general.volume"), not nested.
    keySeparator: false;
  }
}

export async function initI18n(code: string) {
  await i18n.use(initReactI18next).init({
    resources,
    lng: code,
    fallbackLng: "en",
    keySeparator: false,
    // React escapes the values itself.
    interpolation: { escapeValue: false },
  });
  document.documentElement.lang = i18n.language;
}

export default i18n;
