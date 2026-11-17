import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api, type AppInfo, type JournalEntry } from "../../api";
import { Glyph, type GlyphName } from "../icons";
import { dayTitle, useSettings } from "../state";
import { Group, Row, Segmented } from "../ui";

/** New lines show up while the page is open. */
const REREAD_MS = 3000;
/** How long the clear button waits for the second press. */
const CONFIRM_MS = 3000;

const MARKS: Record<JournalEntry["mark"], { glyph: GlyphName; color?: string }> = {
  error: { glyph: "cross", color: "var(--c-red)" },
  warn: { glyph: "warning", color: "var(--c-orange)" },
  info: { glyph: "dot" },
};

export function JournalPage() {
  const { t, i18n } = useTranslation();
  const { info, toast, journalCleared } = useSettings();
  const [view, setView] = useState<"problems" | "all">("problems");
  const [entries, setEntries] = useState<JournalEntry[] | null>(null);
  const [failed, setFailed] = useState("");
  const [copied, setCopied] = useState<string | null>(null);
  // Bumped to read the file again at once.
  const [reads, setReads] = useState(0);
  const [confirming, setConfirming] = useState(false);

  useEffect(() => {
    if (!confirming) return;
    const id = window.setTimeout(() => setConfirming(false), CONFIRM_MS);
    return () => window.clearTimeout(id);
  }, [confirming]);

  // The first press asks, the second clears: the journal can't be brought back.
  const clear = async () => {
    if (!confirming) return setConfirming(true);
    setConfirming(false);
    try {
      await api.clearJournal();
      journalCleared();
      setReads((n) => n + 1);
      toast(t("journal.cleared"));
    } catch (e) {
      toast(t("journal.clear_failed", { error: String(e) }), false);
    }
  };

  useEffect(() => {
    let live = true;
    const read = () =>
      api.readJournal(view === "problems").then(
        (list) => {
          if (!live) return;
          setEntries(list);
          setFailed("");
        },
        (error) => live && setFailed(String(error)),
      );
    void read();
    const id = window.setInterval(() => void read(), REREAD_MS);
    return () => {
      live = false;
      window.clearInterval(id);
    };
  }, [view, reads]);

  const copy = async (entry: JournalEntry) => {
    const ok = await copyText(clipText(entry, info));
    if (!ok) return toast(t("journal.copy_failed"), false);
    setCopied(entry.line);
    window.setTimeout(() => setCopied((line) => (line === entry.line ? null : line)), 1400);
    toast(t("journal.copied_toast"));
  };

  const days = new Map<string, JournalEntry[]>();
  for (const entry of entries ?? []) {
    const day = entry.at.slice(0, 10);
    days.set(day, [...(days.get(day) ?? []), entry]);
  }

  return (
    <>
      <section className="group">
        <Segmented
          value={view}
          options={[
            { value: "problems", label: t("journal.problems") },
            { value: "all", label: t("journal.all") },
          ]}
          onChange={setView}
        />
      </section>

      {failed && <p className="empty bad">{t("journal.read_failed", { error: failed })}</p>}
      {!failed && entries && entries.length === 0 && (
        <p className="empty">{t(view === "problems" ? "journal.no_problems" : "journal.empty")}</p>
      )}

      {[...days].map(([day, list], i) => (
        <section className="group" key={day}>
          <h2 className="gtitle">{dayTitle(new Date(`${day}T00:00:00`), t, i18n.language)}</h2>
          {i === 0 && (
            <div className="jhead">
              <span>{t("journal.time")}</span>
              <span />
              <span>{t("journal.event")}</span>
              <span>{t("journal.repeats")}</span>
            </div>
          )}
          <div className="card">
            {list.map((entry) => (
              <EntryRow
                key={`${entry.line}|${entry.times}`}
                entry={entry}
                copied={copied === entry.line}
                onCopy={() => void copy(entry)}
              />
            ))}
          </div>
        </section>
      ))}

      <Group plain>
        <Row
          label={t("journal.open_file")}
          external
          onClick={() => void api.openJournal().then((opened) => opened || toast(t("journal.open_failed"), false))}
        />
        <Row
          className="danger"
          label={t(confirming ? "journal.clear_confirm" : "journal.clear")}
          onClick={() => void clear()}
        />
      </Group>
    </>
  );
}

function EntryRow({ entry, copied, onCopy }: { entry: JournalEntry; copied: boolean; onCopy: () => void }) {
  const { t } = useTranslation();
  const mark = MARKS[entry.mark];
  return (
    <div className={`jrow ${entry.mark} ${copied ? "copied" : ""}`} role="button" onClick={onCopy}>
      <span className="jtime">{entry.at.slice(11, 16)}</span>
      <span className="jmark" style={mark.color ? { background: mark.color } : undefined}>
        <Glyph name={mark.glyph} size={13} />
      </span>
      <span className="jtitle">{entry.title}</span>
      <span className="jend">
        {entry.times > 1 && <span className="times">×{entry.times}</span>}
        <span className="jcopy">{t(copied ? "journal.copied" : "journal.copy")}</span>
      </span>
      {entry.code && (
        <div className="jsub">
          <div className="why">
            <span className="code">{entry.code}</span>
            <span>{entry.reason}</span>
          </div>
          {entry.detail && <div className="detail">{entry.detail}</div>}
        </div>
      )}
    </div>
  );
}

/** The version and the Windows build first: a copied line is useless without them. */
function clipText(entry: JournalEntry, info: AppInfo | null): string {
  const head = info ? `Great Island ${info.version} · Windows ${info.windows}\n` : "";
  return `${head}${entry.line}${entry.times > 1 ? ` (×${entry.times})` : ""}`;
}

async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // The async clipboard can be refused; the old way still works in a click.
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.append(area);
    area.select();
    const ok = document.execCommand("copy");
    area.remove();
    return ok;
  }
}
