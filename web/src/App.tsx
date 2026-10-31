import { useCallback, useEffect, useState } from "react";
import { DEFAULT_SETTINGS, Lecture, Settings, State, getSettings, getState, saveSettings } from "./api";
import { useRoute } from "./nav";
import { CoursePage, Home, PlayPage } from "./courses";
import { BooksPage, PapersPage, PdfPage } from "./papers";
import { ManagePage } from "./manage";

// ---------- app shell ----------

export default function App() {
  const route = useRoute();
  const [state, setState] = useState<State | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [settingsLoaded, setSettingsLoaded] = useState(false);
  const fontPx = settings.font;
  const theme = settings.theme;

  // Settings are stored by the server (ui-settings.json), not in the browser, so they survive app restarts.
  useEffect(() => {
    getSettings().then((s) => {
      setSettings(s);
      setSettingsLoaded(true);
    });
  }, []);
  const update = useCallback((p: Partial<Settings>) => {
    setSettings((s) => ({ ...s, ...p }));
    saveSettings(p);
  }, []);
  const setFontPx = (n: number) => update({ font: n });
  const setTheme = (t: Settings["theme"]) => update({ theme: t });

  useEffect(() => {
    document.documentElement.style.setProperty("--fs", `${fontPx}px`);
  }, [fontPx]);
  useEffect(() => {
    if (theme === "auto") document.documentElement.removeAttribute("data-theme");
    else document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);

  const refresh = useCallback(async () => {
    try {
      setState(await getState());
      setError(null);
    } catch (e) {
      setError(String((e as Error).message ?? e));
    }
  }, []);

  useEffect(() => {
    refresh();
    const f = () => document.visibilityState === "visible" && refresh();
    document.addEventListener("visibilitychange", f);
    addEventListener("focus", f);
    return () => {
      document.removeEventListener("visibilitychange", f);
      removeEventListener("focus", f);
    };
  }, [refresh]);

  // Update one lecture in memory right away so ticks and bars move without waiting for a reload.
  const patch = useCallback((drive: string, course: string, key: string, p: Partial<Lecture>) => {
    setState((s) => {
      if (!s) return s;
      return {
        ...s,
        courses: s.courses.map((c) => {
          if (c.drive !== drive || c.course !== course) return c;
          const lectures = c.lectures.map((l) => (l.key === key ? { ...l, ...p } : l));
          return { ...c, lectures, done_count: lectures.filter((l) => l.done).length };
        }),
      };
    });
  }, []);

  const patchSpeed = useCallback((drive: string, course: string, speed: number) => {
    setState((s) => (s ? { ...s, courses: s.courses.map((c) => (c.drive === drive && c.course === course ? { ...c, speed } : c)) } : s));
  }, []);

  return (
    <>
      <header className="top">
        <a className="brand" href="#/">CourseShelf</a>
        <nav className="nav">
          <a href="#/" className={route.page === "home" || route.page === "course" || route.page === "play" ? "cur" : ""}>Courses</a>
          <a href="#/papers" className={route.page === "papers" || (route.page === "pdf" && route.kind !== "book" && route.kind !== "collection") ? "cur" : ""}>Papers</a>
          <a href="#/books" className={route.page === "books" || (route.page === "pdf" && route.kind === "book") ? "cur" : ""}>Books</a>
          <a href="#/paperbooks" className={route.page === "paperbooks" || (route.page === "pdf" && route.kind === "collection") ? "cur" : ""}>Paper Books</a>
          <a href="#/manage" className={route.page === "manage" ? "cur" : ""}>Downloads</a>
        </nav>
        <span className="spacer" />
        <span className="grp" aria-label="Text size">
          <button className="btn" onClick={() => setFontPx(Math.max(15, fontPx - 1))} aria-label="Smaller text" title="Smaller text">A−</button>
          <button className="btn" onClick={() => setFontPx(Math.min(30, fontPx + 1))} aria-label="Larger text" title="Larger text">A+</button>
        </span>
        <button className="btn" onClick={() => setTheme(theme === "auto" ? "dark" : theme === "dark" ? "light" : "auto")} title="Colour theme">
          Theme: {theme}
        </button>
        <button className="btn" onClick={refresh} title="Re-read the drives and courses.csv">Refresh</button>
      </header>
      <main className={route.page === "pdf" ? "wide" : undefined}>
        {error && <div className="banner">{error}</div>}
        {state?.error && <div className="banner">{state.error}</div>}
        {(!state || !settingsLoaded) && !error && <p className="muted">Loading…</p>}
        {state && settingsLoaded && route.page === "home" && <Home state={state} onChanged={refresh} />}
        {settingsLoaded && route.page === "papers" && <PapersPage />}
        {settingsLoaded && route.page === "books" && <BooksPage mode="books" />}
        {settingsLoaded && route.page === "paperbooks" && <BooksPage mode="paperbooks" />}
        {settingsLoaded && route.page === "manage" && <ManagePage onChanged={refresh} />}
        {settingsLoaded && route.page === "pdf" && <PdfPage kind={route.kind} refId={route.ref} />}
        {state && settingsLoaded && route.page === "course" && <CoursePage state={state} drive={route.drive} course={route.course} patch={patch} />}
        {state && settingsLoaded && route.page === "play" && (
          <PlayPage state={state} drive={route.drive} course={route.course} file={route.file} patch={patch} patchSpeed={patchSpeed} autoNext={settings.autonext} setAutoNext={(v) => update({ autonext: v })} />
        )}
      </main>
    </>
  );
}
