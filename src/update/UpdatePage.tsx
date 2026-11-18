import { useCallback, useEffect, useLayoutEffect, useRef, useState, type MouseEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useTranslation } from "react-i18next";
import { api, type UpdateView } from "../api";
import { AppIcon } from "../AppIcon";
import "./update.css";

// The notes are the changelog's text: "### Heading" and "- item" lines are all it uses.
function Notes({ text }: { text: string }) {
  const lines = text.split(/\r?\n/).filter((line) => line.trim() !== "");
  return (
    <>
      {lines.map((line, i) => {
        if (line.startsWith("### ")) return <b key={i}>{line.slice(4)}</b>;
        if (line.startsWith("- ")) return <li key={i}>{line.slice(2).replace(/`/g, "")}</li>;
        return <p key={i}>{line.replace(/`/g, "")}</p>;
      })}
    </>
  );
}

/** The update window shows the island's status and sends it commands; it decides and does nothing itself. */
export function UpdatePage() {
  const { t, i18n } = useTranslation();
  const [view, setView] = useState<UpdateView | null>(null);
  const [shown, setShown] = useState(false);
  const [leaving, setLeaving] = useState(false);
  // A button was pressed: the island has not answered yet.
  const [pressed, setPressed] = useState(false);
  // The height of the page with the choices: the pages after it keep it, so the window does not jump.
  const [floor, setFloor] = useState(0);
  const card = useRef<HTMLDivElement>(null);
  const notes = useRef<HTMLDivElement>(null);
  const idle = useRef(0);
  // The notes' edges fade where there is more to scroll.
  const [fade, setFade] = useState({ top: false, bottom: false });
  const measureFade = useCallback(() => {
    const el = notes.current;
    if (!el) return;
    setFade({ top: el.scrollTop > 2, bottom: el.scrollTop + el.clientHeight < el.scrollHeight - 2 });
  }, []);

  useEffect(() => {
    const poll = () => void api.updateStatus().then(setView);
    poll();
    const id = window.setInterval(poll, 300);
    return () => window.clearInterval(id);
  }, []);

  const status = view?.status;
  const offer = status?.offer ?? null;
  const phase = status?.phase;
  const choosing = phase === "available" && offer !== null;
  const working = phase === "downloading" || phase === "verifying" || phase === "installing";
  const failed = phase === "installFailed";

  const leave = useCallback(() => {
    setLeaving(true);
    window.setTimeout(() => void api.updateExit(), 180);
  }, []);

  // Nothing to show for a moment (the answer was taken, the island left): the window goes.
  useEffect(() => {
    if (!view) return;
    const nothing = !choosing && !working && !failed;
    idle.current = nothing || !view.running ? idle.current + 1 : 0;
    if (idle.current >= 3) leave();
  }, [view, choosing, working, failed, leave]);

  const postpone = useCallback(() => {
    void api.updateCommand("postpone");
    leave();
  }, [leave]);

  useEffect(() => {
    if (!choosing || !offer?.canPostpone) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && postpone();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [choosing, offer?.canPostpone, postpone]);

  // The window takes the height of the page and stays in the middle of the screen; it appears with the first fit.
  useLayoutEffect(() => {
    if (!card.current) return;
    // offsetHeight, not getBoundingClientRect: until shown the card is scaled down and the window would be cut short.
    const height = card.current.offsetHeight;
    setFloor((before) => (choosing && height > before ? height : before));
    // The window is shown by this call. Two frames later the hidden look has been painted on screen,
    // so the fade-in runs where it can be seen instead of before the window appears.
    void api.updateFit(height).then(() => requestAnimationFrame(() => requestAnimationFrame(() => setShown(true))));
    measureFade();
  }, [choosing, working, failed, offer?.latest, measureFade]);

  // Dragged by any empty spot; buttons and the notes' text keep their clicks.
  const drag = (e: MouseEvent) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest("button, .notes")) return;
    void getCurrentWindow().startDragging();
  };

  if (!status) return null;

  const lang = i18n.language.startsWith("ru") ? "ru" : "en";
  const notesText = offer ? offer.notes[lang] || offer.notes.en || offer.notes.ru : "";
  const mb = (bytes: number) => (bytes / (1024 * 1024)).toFixed(1).replace(/\.0$/, "");
  const share = status.total > 0 ? Math.min(100, (status.done / status.total) * 100) : 0;
  const version = offer?.latest ?? "";

  return (
    <div
      className={`win ${shown ? "shown" : ""} ${leaving ? "out" : ""}`}
      ref={card}
      style={floor > 0 && !choosing ? { minHeight: floor } : undefined}
      onMouseDown={drag}
    >
      {choosing && offer.canPostpone && (
        <button className="close" onClick={postpone} aria-label={t("update.postpone")}>
          ×
        </button>
      )}
      <div className="head">
        <AppIcon className="app" />
        <div>
          <h3>
            {choosing && t("update.title")}
            {phase === "downloading" && t("update.downloading", { version })}
            {phase === "verifying" && t("update.verifying")}
            {phase === "installing" && t("update.installing", { version })}
            {failed && t("update.failed")}
          </h3>
          <div className="ver">
            {choosing && t("update.versions", { latest: offer.latest, current: offer.current })}
            {phase === "installing" && t("update.installing_hint")}
            {failed && (status.error ?? "")}
          </div>
        </div>
      </div>

      {choosing && (
        <>
          <div className={`notes-wrap ${fade.top ? "fade-top" : ""} ${fade.bottom ? "fade-bottom" : ""}`}>
            <div className="notes" ref={notes} onScroll={measureFade}>
              <b className="title">{t("update.notes")}</b>
              <ul>
                <Notes text={notesText} />
              </ul>
            </div>
          </div>
          <div className="actions">
            <button
              className="btn main grow"
              disabled={pressed}
              onClick={() => {
                setPressed(true);
                void api.updateCommand("install");
              }}
            >
              {t("update.install")}
            </button>
          </div>
          {(offer.canPostpone || offer.canSkip) && (
            <div className="actions">
              {offer.canPostpone && (
                <button className="btn grow" disabled={pressed} onClick={postpone}>
                  {t("update.postpone")}
                </button>
              )}
              {offer.canSkip && (
                <button
                  className="btn grow"
                  disabled={pressed}
                  onClick={() => {
                    void api.updateCommand("skip");
                    leave();
                  }}
                >
                  {t("update.skip")}
                </button>
              )}
            </div>
          )}
        </>
      )}

      {working && (
        <>
          <div className={`bar ${phase === "downloading" && status.total > 0 ? "" : "run"}`}>
            <i style={phase === "downloading" && status.total > 0 ? { width: `${share}%` } : undefined} />
          </div>
          {phase === "downloading" && status.total > 0 && (
            <div className="stat">
              <span>{t("update.size", { done: mb(status.done), total: mb(status.total) })}</span>
              <span>{Math.round(share)} %</span>
            </div>
          )}
        </>
      )}

      {failed && (
        <div className="actions">
          <button className="btn main grow" onClick={leave}>
            {t("update.close")}
          </button>
        </div>
      )}
    </div>
  );
}
