import React from "react";
import ReactDOM from "react-dom/client";
import { api } from "./api";
import { initI18n } from "./i18n";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { SettingsPage } from "./settings/SettingsPage";
import { UpdatePage } from "./update/UpdatePage";
import "./base.css";

// The settings window and the update window use the web layer; the island is native.
const updating = getCurrentWindow().label === "update";
document.documentElement.dataset.window = updating ? "update" : "settings";

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
        {updating ? <UpdatePage /> : <SettingsPage />}
      </React.StrictMode>,
    );
  });
