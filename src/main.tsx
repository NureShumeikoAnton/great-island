import React from "react";
import ReactDOM from "react-dom/client";
import { api } from "./api";
import { initI18n } from "./i18n";
import { SettingsPage } from "./settings/SettingsPage";
import "./base.css";

// Only the settings window uses the web layer; the island and its dialogs are native.
document.documentElement.dataset.window = "settings";

document.addEventListener("contextmenu", (e) => e.preventDefault());

// The language is known before the first paint: no flash of the wrong one.
void api
  .currentLanguage()
  // No answer from the Rust side: English beats a blank window.
  .catch(() => "en")
  .then(initI18n)
  .then(() => {
    ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
      <React.StrictMode>
        <SettingsPage />
      </React.StrictMode>,
    );
  });
