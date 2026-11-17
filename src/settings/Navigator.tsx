// The pages below stay mounted so each keeps its scroll; they are hidden once covered.

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import { Glyph } from "./icons";

export type PageId =
  | "main"
  | "reminders"
  | "reminder"
  | "sound"
  | "island"
  | "monitor"
  | "player"
  | "shots"
  | "record"
  | "energy"
  | "language"
  | "about"
  | "journal";

export interface Route {
  id: PageId;
  /** A reminder's id for its pages. */
  param?: string;
}

interface Navigation {
  go: (route: Route) => void;
  back: () => void;
}

const NavContext = createContext<Navigation>({ go: () => {}, back: () => {} });

export const useNav = () => useContext(NavContext);

interface Entry {
  route: Route;
  key: number;
}

const DURATION = 260;
const EASING = "cubic-bezier(.2,.8,.2,1)";
const COVERED = { transform: "translateX(-30%)", filter: "brightness(.92)" };
const OPEN = { transform: "none", filter: "brightness(1)" };

export function Navigator({ render }: { render: (route: Route) => { title: string; body: ReactNode } }) {
  const { t } = useTranslation();
  const [stack, setStack] = useState<Entry[]>([{ route: { id: "main" }, key: 0 }]);
  const [leaving, setLeaving] = useState<Entry | null>(null);
  const stackRef = useRef(stack);
  stackRef.current = stack;
  const motion = useRef<"push" | "pop" | null>(null);
  // One move at a time: a second click during a slide is dropped.
  const busy = useRef(false);
  const nextKey = useRef(1);
  const pages = useRef(new Map<number, HTMLDivElement>());

  const go = useCallback((route: Route) => {
    if (busy.current) return;
    busy.current = true;
    motion.current = "push";
    setStack((s) => [...s, { route, key: nextKey.current++ }]);
  }, []);

  const back = useCallback(() => {
    const s = stackRef.current;
    if (busy.current || s.length < 2) return;
    busy.current = true;
    motion.current = "pop";
    setLeaving(s[s.length - 1]);
    setStack(s.slice(0, -1));
  }, []);

  useLayoutEffect(() => {
    const kind = motion.current;
    motion.current = null;
    if (!kind) return;
    const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
    const done = () => {
      busy.current = false;
    };
    if (kind === "push") {
      const top = pages.current.get(stack[stack.length - 1].key);
      const under = pages.current.get(stack[stack.length - 2]?.key);
      if (!top || !under) return done();
      const finished = reduce
        ? top.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 120 }).finished
        : Promise.all([
            top.animate([{ transform: "translateX(100%)" }, { transform: "none" }], {
              duration: DURATION,
              easing: EASING,
            }).finished,
            under.animate([OPEN, COVERED], { duration: DURATION, easing: EASING }).finished,
          ]);
      void finished.then(() => {
        under.style.visibility = "hidden";
        done();
      });
    } else if (leaving) {
      const top = pages.current.get(leaving.key);
      const under = pages.current.get(stack[stack.length - 1].key);
      if (!top || !under) {
        setLeaving(null);
        return done();
      }
      under.style.visibility = "";
      const finished = reduce
        ? top.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 120, fill: "forwards" }).finished
        : Promise.all([
            top.animate([{ transform: "none" }, { transform: "translateX(100%)" }], {
              duration: DURATION,
              easing: EASING,
              fill: "forwards",
            }).finished,
            under.animate([COVERED, OPEN], { duration: DURATION, easing: EASING }).finished,
          ]);
      void finished.then(() => {
        setLeaving(null);
        done();
      });
    }
  }, [stack, leaving]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && back();
    // The mouse's side "back" button.
    const onMouse = (e: MouseEvent) => e.button === 3 && back();
    window.addEventListener("keydown", onKey);
    window.addEventListener("mouseup", onMouse);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("mouseup", onMouse);
    };
  }, [back]);

  const shown = leaving ? [...stack, leaving] : stack;
  return (
    <NavContext.Provider value={{ go, back }}>
      <div className="viewport">
        {shown.map(({ route, key }, i) => {
          const { title, body } = render(route);
          const root = route.id === "main";
          return (
            <div
              key={key}
              className={`page ${i > 0 ? "over" : ""}`}
              ref={(el) => {
                if (el) pages.current.set(key, el);
                else pages.current.delete(key);
              }}
            >
              <header className={`bar ${root ? "root" : ""}`}>
                {!root && (
                  <button className="back" onClick={back} title={t("nav.back")}>
                    <Glyph name="back" size={20} />
                  </button>
                )}
                <span className="ttl">{title}</span>
              </header>
              <div className="body">{body}</div>
            </div>
          );
        })}
      </div>
    </NavContext.Provider>
  );
}
