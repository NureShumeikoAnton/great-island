import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, PRINT_SCREEN, type Hotkey, type KeyCheck } from "../api";
import { Mini } from "./ui";

const MODIFIERS = ["ControlLeft", "ControlRight", "ShiftLeft", "ShiftRight", "AltLeft", "AltRight", "MetaLeft", "MetaRight"];

const isPrintScreen = (key: Hotkey | null) =>
  key !== null && key.code === PRINT_SCREEN.code && !key.ctrl && !key.alt && !key.shift && !key.win;

function held(e: KeyboardEvent): string {
  const names = [
    [e.metaKey, "Win"],
    [e.ctrlKey, "Ctrl"],
    [e.altKey, "Alt"],
    [e.shiftKey, "Shift"],
  ]
    .filter(([on]) => on)
    .map(([, name]) => name);
  return names.length ? `${names.join(" + ")} + …` : "";
}

const same = (a: Hotkey | null, b: Hotkey | null) =>
  a !== null &&
  b !== null &&
  a.code === b.code &&
  a.ctrl === b.ctrl &&
  a.alt === b.alt &&
  a.shift === b.shift &&
  a.win === b.win;

interface Props {
  label: string;
  value: Hotkey | null;
  onChange: (key: Hotkey | null) => void;
  printScreen: boolean;
  /** The app's other keys: the same combination can't be taken twice. */
  others: { key: Hotkey | null; is: "record" | "shot" | "area" }[];
}

const TAKEN_BY = { record: "hotkey.same_as_record", shot: "hotkey.same_as_shot", area: "hotkey.same_as_area" } as const;

/** A row of a settings group: the key, and the buttons to change it. */
export function HotkeyField({ label, value, onChange, printScreen, others }: Props) {
  const { t } = useTranslation();
  const [check, setCheck] = useState<KeyCheck | null>(null);
  const [recording, setRecording] = useState(false);
  const [typed, setTyped] = useState("");
  const [error, setError] = useState("");
  const [releaseFailed, setReleaseFailed] = useState(false);

  const problemText = (problem: NonNullable<KeyCheck["problem"]>, keyLabel: string) => {
    switch (problem) {
      case "unknown":
        return t("hotkey.problem_unknown");
      case "tooPlain":
        return t("hotkey.problem_too_plain", { label: keyLabel });
      case "busy":
        return t("hotkey.problem_busy", { label: keyLabel });
    }
  };

  const recheck = () => {
    if (value) void api.checkHotkey(value).then(setCheck);
    else setCheck(null);
  };
  useEffect(recheck, [value]);

  useEffect(() => {
    if (!recording) return;
    const stop = () => {
      setRecording(false);
      setTyped("");
    };
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      // PrtSc reaches the window only on key release; other keys on key press.
      const done = e.type === "keydown" ? e.code !== "PrintScreen" : e.code === "PrintScreen";
      if (!done || MODIFIERS.includes(e.code)) {
        setTyped(held(e));
        return;
      }
      if (e.code === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
        stop();
        return;
      }
      const key: Hotkey = { code: e.code, ctrl: e.ctrlKey, alt: e.altKey, shift: e.shiftKey, win: e.metaKey };
      const taken = others.find((other) => same(key, other.key));
      if (taken) {
        setError(t(TAKEN_BY[taken.is]));
        setTyped("");
        return;
      }
      void api.checkHotkey(key).then((result) => {
        if (result.problem) {
          setError(problemText(result.problem, result.label));
          setTyped("");
          return;
        }
        setError("");
        onChange(key);
        stop();
      });
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", onKey, true);
    window.addEventListener("blur", stop);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onKey, true);
      window.removeEventListener("blur", stop);
    };
  }, [recording, onChange, others, t]);

  const release = async () => {
    const done = await api.releasePrintScreen();
    setReleaseFailed(!done);
    recheck();
  };

  const shown = recording ? typed || t("hotkey.press") : (check?.label ?? (value ? "…" : t("hotkey.none")));

  const canPrintScreen =
    printScreen && !isPrintScreen(value) && !others.some((other) => same(PRINT_SCREEN, other.key));

  return (
    <div className="row block noicon">
      <div className="line">
        <span className="label">{label}</span>
        <span className={`kbd ${recording ? "recording" : ""} ${!recording && !value ? "none" : ""}`}>{shown}</span>
        <Mini
          onClick={() => {
            setError("");
            setRecording(!recording);
            setTyped("");
          }}
        >
          {recording ? t("hotkey.cancel") : t("hotkey.change")}
        </Mini>
        {!recording && canPrintScreen && <Mini onClick={() => onChange(PRINT_SCREEN)}>PrtSc</Mini>}
        {!recording && value && <Mini onClick={() => onChange(null)}>{t("hotkey.remove")}</Mini>}
      </div>
      {recording && <span className="note">{t("hotkey.recording_hint")}</span>}
      {error && <span className="note warn">{error}</span>}
      {!recording && check?.problem && <span className="note warn">{problemText(check.problem, check.label)}</span>}
      {!recording && printScreen && check?.snipping && (
        <span className="note warn">
          {t("hotkey.snipping")}{" "}
          <button className="link" onClick={() => void release()}>
            {t("hotkey.release")}
          </button>
          {releaseFailed && ` ${t("hotkey.release_failed")}`}
        </span>
      )}
    </div>
  );
}
