import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Single, addSingle, downloadSingle, getSingles, removeSingle, revealSingle, BrowseOut, Course, DEFAULT_SETTINGS, MacInfo, macCopy, macRemove, DlPlan, DriveOut, Job, ManageCourse, ManageState, setDrive, MovePlan, Proposal, adoptApply, adoptPreview, browse, cancelJob, clearLocation, getJobs, getManage, planDownload, planMove, setLocation, startDownload, startMove, Lecture, Paper, PapersState, Settings, State, bankPdfUrl, bookPdfUrl, BooksOut, getBooks, setBookmark, Collection, getCollections, createCollection, addToCollection, renameCollection, reorderCollection, deleteCollection, collectionPdfUrl, openCollection, openBook, downloadPaper, getPapers, inboxPdfUrl, openBank, openExternal, openInbox, openPaper, paperPdfUrl, fmtBytes, fmtHours, fmtTime, getSettings, getState, getSubs, mediaUrl, saveProgress, saveSettings, setDone, setSpeed, LookedUp, LookupOut, NewPaper, addPapers, lookupPapers } from "./api";
import { PdfAnnotator } from "./pdfview";

// ---------- tiny hash router: #/  #/c/<drive>/<course>  #/p/<drive>/<course>/<file> ----------

type Route = { page: "home" } | { page: "manage" } | { page: "papers" } | { page: "books" } | { page: "paperbooks" } | { page: "pdf"; kind: "paper" | "bank" | "inbox" | "book" | "collection"; ref: string } | { page: "course"; drive: string; course: string } | { page: "play"; drive: string; course: string; file: string };

function parseHash(h: string): Route {
  const parts = h.replace(/^#\/?/, "").split("/").map(decodeURIComponent);
  if (parts[0] === "papers") return { page: "papers" };
  if (parts[0] === "manage") return { page: "manage" };
  if (parts[0] === "books") return { page: "books" };
  if (parts[0] === "paperbooks") return { page: "paperbooks" };
  if (parts[0] === "pdf" && parts[1] === "book" && parts[2]) return { page: "pdf", kind: "book", ref: parts[2] };
  if (parts[0] === "pdf" && parts[1] === "collection" && parts[2]) return { page: "pdf", kind: "collection", ref: parts[2] };
  if (parts[0] === "pdf" && parts[1] === "paper" && parts[2]) return { page: "pdf", kind: "paper", ref: parts[2] };
  if (parts[0] === "pdf" && parts[1] === "inbox" && parts[2]) return { page: "pdf", kind: "inbox", ref: parts[2] };
  if (parts[0] === "pdf" && parts[1] === "bank" && parts.length >= 4) return { page: "pdf", kind: "bank", ref: `${parts[2]}/${parts[3]}` };
  if (parts[0] === "c" && parts.length >= 3) return { page: "course", drive: parts[1], course: parts.slice(2).join("/") };
  if (parts[0] === "p" && parts.length >= 4) return { page: "play", drive: parts[1], course: parts.slice(2, -1).join("/"), file: parts[parts.length - 1] };
  return { page: "home" };
}
const enc = (s: string) => s.split("/").map(encodeURIComponent).join("/");
const courseHref = (c: { drive: string; course: string }) => `#/c/${encodeURIComponent(c.drive)}/${enc(c.course)}`;
const playHref = (c: { drive: string; course: string }, file: string) => `#/p/${encodeURIComponent(c.drive)}/${enc(c.course)}/${encodeURIComponent(file)}`;

function useRoute(): Route {
  const [r, setR] = useState<Route>(() => parseHash(location.hash));
  useEffect(() => {
    const f = () => setR(parseHash(location.hash));
    addEventListener("hashchange", f);
    return () => removeEventListener("hashchange", f);
  }, []);
  return r;
}

const keyOf = (c: { drive: string; course: string }) => `${c.drive}\u0000${c.course}`;
const courseSeconds = (c: Course) => c.lectures.reduce((a, l) => a + (l.duration ?? 0), 0);

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

// ---------- home ----------

function CourseCard({ c, mac, job, onChanged, onJob }: { c: Course; mac: MacInfo | null; job?: Job; onChanged: () => void; onJob: () => void }) {
  const total = c.lectures.length;
  const pct = total ? (c.done_count / total) * 100 : 0;
  const secs = courseSeconds(c);
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const macCopyInfo = c.copies.find((x) => x.is_mac && x.lectures > 0);
  const source = c.copies.find((x) => !x.is_mac && x.connected && x.lectures > 0);
  const offline = c.copies.find((x) => !x.is_mac && !x.connected);
  const stale = macCopyInfo && source && source.lectures > macCopyInfo.lectures;
  const room = mac && mac.max_bytes != null ? mac.max_bytes - mac.used_bytes : null;
  const need = source ? source.size_bytes - (macCopyInfo?.size_bytes ?? 0) : 0;
  const tooBig = room != null && need > room;
  const copying = job && (job.status === "running" || job.status === "queued");

  const doCopy = async () => {
    setBusy(true); setMsg(null);
    try { await macCopy(c.course); onJob(); } catch (e) { setMsg(String((e as Error).message ?? e)); } finally { setBusy(false); }
  };
  const doRemove = async () => {
    if (!confirm) { setConfirm(true); setTimeout(() => setConfirm(false), 6000); return; }
    setBusy(true); setMsg(null); setConfirm(false);
    try { const r = await macRemove(c.course); setMsg(`Freed ${fmtGB(r.freed_bytes)} on the Mac. The copy on ${r.kept_copy_on} is kept.`); onChanged(); } catch (e) { setMsg(String((e as Error).message ?? e)); } finally { setBusy(false); }
  };

  return (
    <div className="cardwrap">
      <a className={`card ${c.connected ? "" : "off"}`} href={courseHref(c)}>
        <h3>
          {c.title}
          {!c.listed && <span className="tag" title="Found on the drive but not in courses.csv">not in CSV</span>}
          {c.listed && !c.enabled && <span className="tag">paused</span>}
        </h3>
        <div className="muted small">{c.course.includes("/") ? c.course.split("/").slice(0, -1).join(" / ") : "\u00a0"}</div>
        <div className="bar" aria-hidden><i style={{ width: `${pct}%` }} /></div>
        <div className="meta">
          <span>{total ? `${c.done_count} / ${total} done` : c.connected ? "nothing downloaded yet" : "drive not connected"}</span>
          <span>{total ? fmtHours(secs) : ""}</span>
        </div>
        <div className="chips">
          {c.copies.map((x) => (
            <span key={x.drive + x.location} className={`tag ${x.connected ? "" : "warn"} ${x.is_mac ? "macchip" : ""}`} title={x.location}>
              {x.is_mac ? "Mac" : x.drive}{x.lectures > 0 ? ` · ${fmtGB(x.size_bytes)}` : ""}{x.connected ? "" : " · not connected"}
            </span>
          ))}
        </div>
      </a>
      <div className="cardacts">
        {copying && job ? (
          <div className="copyjob"><div className="bar resume"><i style={{ width: `${Math.min(100, job.percent)}%` }} /></div><span className="small muted">Copying to Mac… {job.percent.toFixed(0)}%</span></div>
        ) : (
          <>
            {!macCopyInfo && source && mac && (
              <button className="btn sm" disabled={busy || tooBig} onClick={doCopy} title={tooBig ? `Needs ${fmtGB(need)}; only ${fmtGB(room)} left under your Mac limit` : `Copy ${fmtGB(source.size_bytes)} from ${source.drive} to this Mac so you can study without the drive`}>
                Copy to Mac · {fmtGB(source.size_bytes)}
              </button>
            )}
            {stale && (
              <button className="btn sm" disabled={busy || tooBig} onClick={doCopy} title="The drive has lectures that are not on the Mac copy yet">Update Mac copy · +{source!.lectures - macCopyInfo!.lectures}</button>
            )}
            {macCopyInfo && source && !stale && (
              <button className={`btn sm ${confirm ? "danger" : ""}`} disabled={busy} onClick={doRemove} title={`Delete the Mac copy (${fmtGB(macCopyInfo.size_bytes)}). It is checked against the copy on ${source.drive} first, and that copy is kept.`}>
                {confirm ? `Click again: delete ${fmtGB(macCopyInfo.size_bytes)} from the Mac` : "Remove from Mac"}
              </button>
            )}
            {macCopyInfo && !source && <span className="small muted">{offline ? `On the Mac. Plug in ${offline.drive} to remove it safely.` : "Only copy: on the Mac."}</span>}
            {!macCopyInfo && !source && offline && <span className="small muted">Plug in {offline.drive} to copy it to the Mac.</span>}
          </>
        )}
        {tooBig && !macCopyInfo && <span className="small" style={{ color: "var(--warn)" }}>Over your Mac limit</span>}
      </div>
      {msg && <div className="small" style={{ color: msg.startsWith("Freed") ? "var(--done)" : "var(--bad)", padding: "0 .3rem" }}>{msg}</div>}
    </div>
  );
}

function MacMeter({ mac }: { mac: MacInfo }) {
  const pct = mac.max_bytes ? Math.min(100, (mac.used_bytes / mac.max_bytes) * 100) : 0;
  return (
    <div className="macmeter" title={`Courses stored on this Mac (${mac.path})`}>
      <span><b>Mac</b> {fmtGB(mac.used_bytes)}{mac.max_bytes ? ` of ${fmtGB(mac.max_bytes)}` : ""}{mac.free_bytes != null ? ` · ${fmtGB(mac.free_bytes)} disk free` : ""}</span>
      {mac.max_bytes ? <div className="bar" aria-hidden><i style={{ width: `${pct}%`, background: pct > 90 ? "var(--warn)" : undefined }} /></div> : null}
    </div>
  );
}

function Home({ state, onChanged }: { state: State; onChanged: () => void }) {
  const [drive, setDrive] = useState<string>("all");
  const [jobs, setJobs] = useState<Job[]>([]);
  const loadJobs = useCallback(() => getJobs().then(setJobs).catch(() => {}), []);
  useEffect(() => { loadJobs(); }, [loadJobs]);
  const active = jobs.some((j) => j.status === "running" || j.status === "queued");
  useEffect(() => {
    if (!active) return;
    const t = setInterval(loadJobs, 1000);
    return () => clearInterval(t);
  }, [active, loadJobs]);
  const wasActive = useRef(false);
  useEffect(() => {
    if (wasActive.current && !active) onChanged();
    wasActive.current = active;
  }, [active, onChanged]);
  const jobFor = (course: string) => jobs.find((j) => j.course === course && (j.status === "running" || j.status === "queued"));
  const courses = state.courses.filter((c) => drive === "all" || c.drive === drive);
  const withVideos = courses.filter((c) => c.lectures.length > 0);
  const pending = courses.filter((c) => c.lectures.length === 0);
  const recent = state.recent.slice(0, 6);
  const courseOf = (d: string, c: string) => state.courses.find((x) => x.drive === d && x.course === c);
  return (
    <>
      <div className="drives">
        <button className={`chip ${drive === "all" ? "" : ""}`} onClick={() => setDrive("all")} aria-pressed={drive === "all"} style={drive === "all" ? { borderColor: "var(--accent)" } : undefined}>
          All drives
        </button>
        {state.drives.map((d) => (
          <button key={d.name} className={`chip ${d.connected ? "" : "off"}`} onClick={() => setDrive(d.name)} style={drive === d.name ? { borderColor: "var(--accent)" } : undefined} title={d.path}>
            <span className="dot" />
            {d.name}
            {d.connected ? ` · ${fmtBytes(d.free_bytes)} free of ${fmtBytes(d.total_bytes)}` : " · not connected"}
          </button>
        ))}
      </div>

      {state.mac && <MacMeter mac={state.mac} />}

      {recent.length > 0 && (
        <>
          <h2>Continue watching</h2>
          <div className="cont">
            {recent.map((r) => {
              const c = courseOf(r.drive, r.course);
              const pct = r.duration > 0 ? (r.position / r.duration) * 100 : 0;
              return (
                <a key={`${r.drive}/${r.course}/${r.key}`} className="card" href={playHref({ drive: r.drive, course: r.course }, r.file)}>
                  <h3>{r.title}</h3>
                  <div className="muted small">{c?.title ?? r.course_title}</div>
                  <div className="bar resume" aria-hidden><i style={{ width: `${pct}%` }} /></div>
                  <div className="meta"><span>resume at {fmtTime(r.position)}</span><span>{r.duration > 0 ? `of ${fmtTime(r.duration)}` : ""}</span></div>
                </a>
              );
            })}
          </div>
        </>
      )}

      <h1 style={{ marginTop: "1.6rem" }}>Courses</h1>
      {withVideos.length === 0 && <div className="empty">No videos found on the connected drives yet. Add rows to courses.csv and run <code>courseshelf-fetch run</code>.</div>}
      <div className="grid">{withVideos.map((c) => <CourseCard key={keyOf(c)} c={c} mac={state.mac} job={jobFor(c.course)} onChanged={onChanged} onJob={loadJobs} />)}</div>

      <SavedVideos />

      {pending.length > 0 && (
        <>
          <h2>Listed in courses.csv, nothing on disk</h2>
          <div className="grid">{pending.map((c) => <CourseCard key={keyOf(c)} c={c} mac={state.mac} job={jobFor(c.course)} onChanged={onChanged} onJob={loadJobs} />)}</div>
        </>
      )}
    </>
  );
}

function SavedVideos() {
  const [items, setItems] = useState<Single[]>([]);
  useEffect(() => { getSingles().then((x) => setItems(x.items)).catch(() => {}); }, []);
  if (items.length === 0) return null;
  return (
    <>
      <h2>Saved video links</h2>
      <p className="muted small">Links you added under Downloads → Single videos. A video appears under Courses once it is downloaded into a folder inside one of your drives.</p>
      <div className="grid">
        {items.map((it) => (
          <div key={it.id} className="card">
            <h3>{it.title}</h3>
            <div className="muted small">{it.uploader}{it.duration ? ` · ${fmtTime(it.duration)}` : ""}</div>
            <div className="meta">
              <span className={`badge ${it.status === "downloaded" ? "ok" : "info"}`}>{it.status === "downloaded" ? "downloaded" : "not downloaded yet"}</span>
              {it.status === "downloaded" ? <button className="btn sm" onClick={() => revealSingle(it.id)}>Show in Finder</button> : <a className="btn sm" href="#/manage">Download…</a>}
            </div>
            {it.status === "downloaded" && it.dest && <div className="muted small">{it.dest}</div>}
          </div>
        ))}
      </div>
    </>
  );
}

// ---------- tick button ----------

function Tick({ on, onClick, label }: { on: boolean; onClick: () => void; label: string }) {
  return (
    <button className={`tick ${on ? "on" : ""}`} onClick={onClick} aria-pressed={on} aria-label={label} title={label}>
      {on ? "✓" : ""}
    </button>
  );
}

// ---------- course page ----------

function CoursePage({ state, drive, course, patch }: { state: State; drive: string; course: string; patch: (d: string, c: string, k: string, p: Partial<Lecture>) => void }) {
  const c = state.courses.find((x) => x.drive === drive && x.course === course);
  if (!c) return <div className="empty">That course is not in the library. <a href="#/">Back</a></div>;
  const total = c.lectures.length;
  const next = c.lectures.find((l) => !l.done);
  const toggle = async (l: Lecture) => {
    patch(drive, course, l.key, { done: !l.done });
    try {
      await setDone(drive, course, l.key, !l.done);
    } catch {
      patch(drive, course, l.key, { done: l.done });
    }
  };
  return (
    <>
      <div className="crumbs"><a href="#/">Courses</a> / {c.title}</div>
      <h1>{c.title}</h1>
      <p className="muted">
        {c.done_count} of {total} done · {fmtHours(courseSeconds(c))} · drive {c.drive}
        {c.note && <> · {c.note}</>}
        {c.playlist_url && <> · <a href={c.playlist_url} onClick={(e) => { e.preventDefault(); openExternal(c.playlist_url); }}>playlist</a></>}
      </p>
      {next && <p><a className="btn primary" href={playHref(c, next.file)} style={{ textDecoration: "none", display: "inline-block" }}>{next.position > 5 ? "Resume" : "Start"}: {next.title}</a></p>}
      {total === 0 ? (
        <div className="empty">{c.connected ? "Nothing downloaded for this course yet." : `Drive “${c.drive}” is not connected.`}</div>
      ) : (
        <table className="lec">
          <thead><tr><th>Done</th><th>#</th><th>Lecture</th><th>Length</th><th>Quality</th><th>Watched</th></tr></thead>
          <tbody>
            {c.lectures.map((l) => (
              <tr className="row" key={l.key}>
                <td><Tick on={l.done} onClick={() => toggle(l)} label={l.done ? "Mark as not done" : "Mark as done"} /></td>
                <td className="num">{l.index || ""}</td>
                <td className="t"><a href={playHref(c, l.file)}>{l.title}</a></td>
                <td>{fmtTime(l.duration)}</td>
                <td className="muted small">{l.height ? `${l.height}p` : ""}</td>
                <td style={{ minWidth: "7rem" }}>
                  {l.done ? <span className="muted small">finished</span> : l.position > 5 ? (
                    <><div className="bar resume" aria-hidden style={{ marginTop: 0 }}><i style={{ width: `${l.duration ? (l.position / l.duration) * 100 : 0}%` }} /></div><span className="muted small">at {fmtTime(l.position)}</span></>
                  ) : <span className="muted small">not started</span>}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </>
  );
}

// ---------- player ----------

const SPEEDS = [0.75, 1, 1.25, 1.5, 1.75, 2];

function PlayPage({ state, drive, course, file, patch, patchSpeed, autoNext, setAutoNext }: {
  state: State; drive: string; course: string; file: string;
  patch: (d: string, c: string, k: string, p: Partial<Lecture>) => void;
  patchSpeed: (d: string, c: string, s: number) => void;
  autoNext: boolean; setAutoNext: (v: boolean) => void;
}) {
  const c = state.courses.find((x) => x.drive === drive && x.course === course);
  const idx = c ? c.lectures.findIndex((l) => l.file === file) : -1;
  const lec: Lecture | undefined = c && idx >= 0 ? c.lectures[idx] : undefined;
  if (!c || !lec) return <div className="empty">That lecture is not on a connected drive. <a href="#/">Back</a></div>;
  return <Player key={`${drive}/${course}/${file}`} c={c} lec={lec} prev={c.lectures[idx - 1]} next={c.lectures[idx + 1]} patch={patch} patchSpeed={patchSpeed} autoNext={autoNext} setAutoNext={setAutoNext} />;
}

function Player({ c, lec, prev, next, patch, patchSpeed, autoNext, setAutoNext }: {
  c: Course; lec: Lecture; prev?: Lecture; next?: Lecture;
  patch: (d: string, c: string, k: string, p: Partial<Lecture>) => void;
  patchSpeed: (d: string, c: string, s: number) => void;
  autoNext: boolean; setAutoNext: (v: boolean) => void;
}) {
  const vref = useRef<HTMLVideoElement>(null);
  const lastSaved = useRef<number>(-1);
  const [failed, setFailed] = useState<string | null>(null);
  const [subsUrl, setSubsUrl] = useState<string | null>(null);
  const [subsOn, setSubsOn] = useState(false);
  const [speed, setSpeedState] = useState(c.speed || 1);
  const lecRef = useRef(lec);
  lecRef.current = lec;

  const save = useCallback(
    (keepalive = false) => {
      const v = vref.current;
      if (!v || !isFinite(v.duration) || v.duration <= 0) return;
      const pos = v.currentTime;
      if (Math.abs(pos - lastSaved.current) < 0.5) return;
      lastSaved.current = pos;
      const l = lecRef.current;
      patch(c.drive, c.course, l.key, { position: pos, duration: v.duration, last_watched: Math.floor(Date.now() / 1000) });
      saveProgress(c.drive, c.course, l.key, pos, v.duration, keepalive)
        .then((r) => r.done && !l.done && patch(c.drive, c.course, l.key, { done: true }))
        .catch(() => {});
    },
    [c.drive, c.course, patch],
  );

  // Save every 5 s while playing, and when leaving the page.
  useEffect(() => {
    const t = setInterval(() => {
      const v = vref.current;
      if (v && !v.paused) save();
    }, 5000);
    const onHide = () => save(true);
    addEventListener("pagehide", onHide);
    document.addEventListener("visibilitychange", () => document.visibilityState === "hidden" && save(true));
    return () => {
      clearInterval(t);
      removeEventListener("pagehide", onHide);
      save(true);
    };
  }, [save]);

  // Subtitles: fetch the .vtt text and hand it to the <track> as a blob (works regardless of the page's origin rules).
  useEffect(() => {
    let url: string | null = null;
    let alive = true;
    const f = lec.subs.find((s) => /\.en[.-]/.test(s)) ?? lec.subs[0];
    if (f) {
      getSubs(c.drive, c.course, f).then((t) => {
        if (t && alive) {
          url = URL.createObjectURL(new Blob([t], { type: "text/vtt" }));
          setSubsUrl(url);
        }
      });
    }
    return () => {
      alive = false;
      if (url) URL.revokeObjectURL(url);
    };
  }, [c.drive, c.course, lec.subs]);

  useEffect(() => {
    const v = vref.current;
    const tt = v?.textTracks[0];
    if (tt) tt.mode = subsOn ? "showing" : "hidden";
  }, [subsOn, subsUrl]);

  const changeSpeed = (s: number) => {
    setSpeedState(s);
    if (vref.current) vref.current.playbackRate = s;
    patchSpeed(c.drive, c.course, s);
    setSpeed(c.drive, c.course, s).catch(() => {});
  };

  const toggleDone = async () => {
    const want = !lec.done;
    patch(c.drive, c.course, lec.key, { done: want });
    try {
      await setDone(c.drive, c.course, lec.key, want);
    } catch {
      patch(c.drive, c.course, lec.key, { done: !want });
    }
  };

  const go = (l?: Lecture) => {
    if (!l) return;
    save(true);
    location.hash = playHref(c, l.file);
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t && ["INPUT", "SELECT", "TEXTAREA"].includes(t.tagName)) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const v = vref.current;
      if (!v) return;
      const seek = (d: number) => { v.currentTime = Math.max(0, Math.min(v.duration || 1e9, v.currentTime + d)); };
      switch (e.key) {
        case " ": case "k": e.preventDefault(); v.paused ? v.play() : v.pause(); break;
        case "ArrowLeft": e.preventDefault(); seek(-5); break;
        case "ArrowRight": e.preventDefault(); seek(5); break;
        case "j": seek(-10); break;
        case "l": seek(10); break;
        case "[": changeSpeed(Math.max(0.5, +(speed - 0.25).toFixed(2))); break;
        case "]": changeSpeed(Math.min(3, +(speed + 0.25).toFixed(2))); break;
        case "n": go(next); break;
        case "p": go(prev); break;
        case "f": document.fullscreenElement ? document.exitFullscreen() : v.requestFullscreen?.(); break;
        case "c": setSubsOn((x) => !x); break;
        case "d": toggleDone(); break;
        default: return;
      }
    };
    addEventListener("keydown", onKey);
    return () => removeEventListener("keydown", onKey);
  });

  const src = useMemo(() => mediaUrl(c.drive, c.course, lec.file), [c.drive, c.course, lec.file]);

  return (
    <>
      <div className="crumbs"><a href="#/">Courses</a> / <a href={courseHref(c)}>{c.title}</a></div>
      <h1>{lec.index ? `${lec.index}. ` : ""}{lec.title}</h1>
      <div className="player">
        <div className="stage">
          <video
            ref={vref}
            src={src}
            controls
            autoPlay
            preload="metadata"
            onLoadedMetadata={(e) => {
              const v = e.currentTarget;
              v.playbackRate = speed;
              const l = lecRef.current;
              if (l.position > 5 && l.position < v.duration - 5 && !l.done) v.currentTime = l.position;
              lastSaved.current = v.currentTime;
              if (!l.duration || Math.abs((l.duration ?? 0) - v.duration) > 1) patch(c.drive, c.course, l.key, { duration: v.duration });
            }}
            onPause={() => save()}
            onSeeked={() => save()}
            onEnded={() => {
              const v = vref.current;
              if (v) {
                lastSaved.current = -1;
                saveProgress(c.drive, c.course, lec.key, v.duration, v.duration).catch(() => {});
                patch(c.drive, c.course, lec.key, { done: true, position: v.duration });
              }
              if (autoNext && next) setTimeout(() => go(next), 1500);
            }}
            onError={() => {
              const ct = vref.current?.error;
              setFailed(`The video engine could not play this file${lec.vcodec ? ` (codec ${lec.vcodec})` : ""}${ct?.message ? `: ${ct.message}` : ""}. Open it in QuickTime or IINA from Finder — the file is ${lec.file}`);
            }}
          >
            {subsUrl && <track kind="subtitles" srcLang="en" label="English" src={subsUrl} default={subsOn} />}
          </video>
          {failed && <div className="err">{failed}</div>}
          <div className="controls">
            <button className="btn" onClick={() => go(prev)} disabled={!prev}>◀ Previous</button>
            <button className="btn" onClick={() => go(next)} disabled={!next}>Next ▶</button>
            <label className="grp">Speed
              <select value={speed} onChange={(e) => changeSpeed(parseFloat(e.target.value))}>
                {[...new Set([...SPEEDS, speed])].sort((a, b) => a - b).map((s) => <option key={s} value={s}>{s}×</option>)}
              </select>
            </label>
            <button className={`btn ${subsOn ? "on" : ""}`} onClick={() => setSubsOn(!subsOn)} disabled={!subsUrl} title={subsUrl ? "Toggle captions (c)" : "No captions for this lecture"}>
              Captions {subsOn ? "on" : "off"}
            </button>
            <button className={`btn ${autoNext ? "on" : ""}`} onClick={() => setAutoNext(!autoNext)}>Autoplay next {autoNext ? "on" : "off"}</button>
            <button className={`btn ${lec.done ? "on" : ""}`} onClick={toggleDone}>{lec.done ? "✓ Done" : "Mark done"}</button>
          </div>
          <div className="keys">
            Space/K play-pause · ←/→ 5 s · J/L 10 s · [ ] speed · N/P next/previous · F fullscreen · C captions · D done
            {lec.height ? ` · ${lec.width ?? ""}×${lec.height}` : ""}{lec.size_bytes ? ` · ${fmtBytes(lec.size_bytes)}` : ""}
          </div>
        </div>
        <aside className="side" aria-label="Lectures in this course">
          {c.lectures.map((l) => (
            <a key={l.key} href={playHref(c, l.file)} className={l.file === lec.file ? "cur" : ""}>
              <span className={`mini ${l.done ? "on" : ""}`} aria-hidden />
              <span className="lt">
                <span>{l.index ? `${l.index}. ` : ""}{l.title}</span>
                <span className="muted small">{fmtTime(l.duration)}{!l.done && l.position > 5 ? ` · at ${fmtTime(l.position)}` : ""}</span>
              </span>
            </a>
          ))}
        </aside>
      </div>
    </>
  );
}


// ---------- papers ----------

const PAGE = 100;

function terms(q: string): string[] {
  return [...q.matchAll(/"([^"]+)"|(\S+)/g)].map((m) => (m[1] ?? m[2]).toLowerCase()).filter(Boolean);
}

type StatusFilter = "all" | "printed" | "unprinted" | "held" | "linkonly" | "bank" | "read" | "broken";

const STATUS_LABEL: Record<StatusFilter, string> = {
  all: "Any status",
  printed: "Printed",
  unprinted: "Not printed",
  held: "PDF in library",
  linkonly: "Link only (no PDF)",
  bank: "Has question bank",
  read: "Marked read",
  broken: "Broken local link",
};

function PapersPage() {
  const [data, setData] = useState<PapersState | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [q, setQ] = useState("");
  const [section, setSection] = useState("all");
  const [stage, setStage] = useState("all");
  const [topic, setTopic] = useState("all");
  const [status, setStatus] = useState<StatusFilter>("all");
  const [shown, setShown] = useState(PAGE);
  const [open, setOpen] = useState<string | null>(null);
  const [sel, setSel] = useState<Set<string>>(new Set());
  const [adding, setAdding] = useState(false);
  const toggleSel = (id: string) => setSel((s) => { const n = new Set(s); if (n.has(id)) n.delete(id); else n.add(id); return n; });

  const load = useCallback(() => {
    getPapers().then(setData).catch((e) => setErr(String(e.message ?? e)));
  }, []);
  useEffect(load, [load]);

  const papers = data?.papers ?? [];
  const stages = useMemo(() => [...new Set(papers.map((p) => p.stage).filter(Boolean))], [papers]);
  const topics = useMemo(() => [...new Set(papers.map((p) => p.topic).filter(Boolean))].sort(), [papers]);
  const t = useMemo(() => terms(q), [q]);

  const filtered = useMemo(() => {
    return papers.filter((p) => {
      if (section !== "all" && !p.section.startsWith(section)) return false;
      if (stage !== "all" && p.stage !== stage) return false;
      if (topic !== "all" && p.topic !== topic) return false;
      switch (status) {
        case "printed": if (!p.printed) return false; break;
        case "unprinted": if (p.printed) return false; break;
        case "held": if (!p.held) return false; break;
        case "linkonly": if (p.held) return false; break;
        case "bank": if (p.banks.length === 0) return false; break;
        case "read": if (!p.date_read) return false; break;
        case "broken": if (!p.missing_local) return false; break;
      }
      if (t.length) {
        const hay = `${p.sno} ${p.title} ${p.topic} ${p.stage} ${p.projects} ${p.notes} ${p.link} ${p.arxiv ?? ""}`.toLowerCase();
        if (!t.every((x) => hay.includes(x))) return false;
      }
      return true;
    });
  }, [papers, section, stage, topic, status, t]);

  useEffect(() => setShown(PAGE), [q, section, stage, topic, status]);

  if (err) return <div className="banner">{err}</div>;
  if (!data) return <p className="muted">Loading papers…</p>;
  if (!data.available) return <div className="banner">Papers are not available: {data.error}</div>;

  const held = papers.filter((p) => p.held).length;
  const printed = papers.filter((p) => p.printed).length;
  const banks = papers.filter((p) => p.banks.length > 0).length;
  const read = papers.filter((p) => p.date_read).length;

  return (
    <>
      <h1>Papers</h1>
      <div className="stats">
        <div className="stat"><b>{papers.length}</b><span>in the database</span></div>
        <div className="stat"><b>{held}</b><span>PDF in library</span></div>
        <div className="stat"><b>{printed}</b><span>printed</span></div>
        <div className="stat"><b>{banks}</b><span>with question bank</span></div>
        <div className="stat"><b>{read}</b><span>marked read</span></div>
      </div>

      <div className="filters">
        <input className="search" type="search" placeholder='Search title, topic, project, note, arXiv id… ("quoted phrase" works)' value={q} onChange={(e) => setQ(e.target.value)} aria-label="Search papers" />
        <select value={section} onChange={(e) => setSection(e.target.value)} aria-label="Section">
          <option value="all">Both sections</option>
          <option value="A">A · Curriculum</option>
          <option value="B">B · Research &amp; Reference</option>
        </select>
        <select value={stage} onChange={(e) => setStage(e.target.value)} aria-label="Stage">
          <option value="all">Any stage</option>
          {stages.map((s) => <option key={s} value={s}>{s.length > 60 ? s.slice(0, 58) + "…" : s}</option>)}
        </select>
        <select value={topic} onChange={(e) => setTopic(e.target.value)} aria-label="Topic">
          <option value="all">Any topic</option>
          {topics.map((s) => <option key={s} value={s}>{s}</option>)}
        </select>
        <select value={status} onChange={(e) => setStatus(e.target.value as StatusFilter)} aria-label="Status">
          {(Object.keys(STATUS_LABEL) as StatusFilter[]).map((k) => <option key={k} value={k}>{STATUS_LABEL[k]}</option>)}
        </select>
        <button className="btn" onClick={load} title="Re-read the paper list">Reload</button>
        <button className="btn primary" onClick={() => setAdding(!adding)} aria-expanded={adding}>{adding ? "Close" : "Add papers"}</button>
      </div>
      {adding && <AddPapers onAdded={load} onClose={() => setAdding(false)} />}
      {data.starter
        ? <p className="muted small">{filtered.length} of {papers.length} shown · your paper list is kept in this computer's CourseShelf data folder (<code>{data.list}</code>). To use a paper library of your own instead, add a <code>[papers]</code> section to <code>config.toml</code>.</p>
        : <p className="muted small">{filtered.length} of {papers.length} shown · the paper list (<code>[papers]</code> in <code>config.toml</code>); CourseShelf changes it only through <b>Add papers</b>.</p>}

      <div className="controls">
        <button className="btn sm" onClick={() => setSel(new Set([...sel, ...filtered.map((p) => p.id)]))} disabled={filtered.length === 0} title="Tick every paper that matches the search and filters">Select all {filtered.length} shown</button>
        {sel.size > 0 && <button className="btn sm" onClick={() => setSel(new Set())}>Clear selection</button>}
        <span className="muted small">{sel.size} selected</span>
      </div>
      {sel.size > 0 && <BookBuilder papers={papers} sel={sel} defaultTitle={q.trim()} onDone={() => { load(); }} onClear={() => setSel(new Set())} />}

      {papers.length === 0 ? <div className="empty">Your paper list is empty. Press <b>Add papers</b> and paste arXiv ids, arXiv links or DOIs to start it.</div>
        : filtered.length === 0 ? <div className="empty">No paper matches these filters.</div> : (
        <table className="lec papers">
          <thead><tr><th aria-label="Select"></th><th>#</th><th>Title</th><th>Status</th><th>Open</th></tr></thead>
          <tbody>
            {filtered.slice(0, shown).map((p) => <PaperRow key={p.id} p={p} checked={sel.has(p.id)} onCheck={() => toggleSel(p.id)} expanded={open === p.id} toggle={() => setOpen(open === p.id ? null : p.id)} onChanged={load} />)}
          </tbody>
        </table>
      )}
      {shown < filtered.length && (
        <p style={{ textAlign: "center" }}><button className="btn" onClick={() => setShown(shown + PAGE)}>Show {Math.min(PAGE, filtered.length - shown)} more</button></p>
      )}
    </>
  );
}

/** Paste arXiv ids / links / DOIs, look them up, choose a topic and a note for each, and add them to the paper list (as a job). */
function AddPapers({ onAdded, onClose }: { onAdded: () => void; onClose: () => void }) {
  const [text, setText] = useState("");
  const [res, setRes] = useState<LookupOut | null>(null);
  const [rows, setRows] = useState<(NewPaper & { pick: boolean; existing: LookedUp["existing"] })[]>([]);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [job, setJob] = useState<Job | null>(null);
  const running = job && (job.status === "running" || job.status === "queued");
  const note = (p: NewPaper) => {
    const who = p.authors.length ? `${p.authors[0].split(" ").slice(-1)[0]}${p.authors.length > 1 ? " et al." : ""}` : "";
    return [who && p.year ? `${who}, ${p.year}` : who || p.year, p.venue].filter(Boolean).join(". ") + (who || p.year || p.venue ? ". " : "");
  };
  const look = async () => {
    setErr(null); setBusy(true); setJob(null);
    try {
      const r = await lookupPapers(text);
      setRes(r);
      setRows(r.found.map((f) => ({ ...f.paper, topic: "", notes: note(f.paper), pick: !f.existing, existing: f.existing })));
    } catch (e) { setErr(String((e as Error).message ?? e)); setRes(null); } finally { setBusy(false); }
  };
  useEffect(() => {
    if (!job || !(job.status === "running" || job.status === "queued")) return;
    const t = setInterval(() => getJobs().then((l) => { const j = l.find((x) => x.id === job.id); if (j) { setJob(j); if (j.status !== "running" && j.status !== "queued") onAdded(); } }).catch(() => {}), 1000);
    return () => clearInterval(t);
  }, [job?.id, job?.status]); // eslint-disable-line react-hooks/exhaustive-deps
  const chosen = rows.filter((r) => r.pick && !r.existing);
  const add = async () => {
    setErr(null); setBusy(true);
    try {
      const r = await addPapers(chosen.map(({ pick: _p, existing: _e, ...p }) => p)); // eslint-disable-line @typescript-eslint/no-unused-vars
      const l = await getJobs();
      setJob(l.find((x) => x.id === r.job) ?? null);
    } catch (e) { setErr(String((e as Error).message ?? e)); } finally { setBusy(false); }
  };
  const set = (i: number, patch: Partial<NewPaper & { pick: boolean }>) => setRows(rows.map((r, k) => (k === i ? { ...r, ...patch } : r)));
  return (
    <div className="mcourse">
      <b>Add papers to the list</b>
      <p className="small muted" style={{ margin: ".3rem 0" }}>Paste arXiv ids or links (2307.03172, arxiv.org/abs/…, arxiv.org/pdf/…) or DOIs (10.1038/…, doi.org/…), any separator.
        They are looked up on arXiv / Crossref; papers already in the list are marked and cannot be added twice. Nothing is written until you press Add.</p>
      <textarea className="search" style={{ width: "100%", minHeight: "5.5rem", fontFamily: "inherit" }} value={text} onChange={(e) => setText(e.target.value)} placeholder="https://arxiv.org/abs/2307.03172&#10;2404.16130&#10;10.1038/323533a0" aria-label="Papers to add" disabled={!!running} />
      <div className="controls">
        <button className="btn primary" onClick={look} disabled={busy || !!running || !text.trim()}>{busy && !rows.length ? "Looking up…" : "Look up"}</button>
        <button className="btn sm" onClick={onClose} disabled={!!running}>Close</button>
      </div>
      {err && <div className="err">{err}</div>}
      {res && (res.errors.length > 0 || res.not_understood.length > 0) && (
        <div className="small" style={{ color: "var(--warn)" }}>
          {res.errors.map((e) => <div key={e}>{e}</div>)}
          {res.not_understood.length > 0 && <div>Not an arXiv id or DOI: {res.not_understood.join(", ")}</div>}
        </div>
      )}
      {rows.length > 0 && (
        <>
          <datalist id="addpaper-topics">{(res?.topics ?? []).map((t) => <option key={t} value={t} />)}</datalist>
          <table className="lec papers">
            <thead><tr><th aria-label="Add"></th><th style={{ width: "48%" }}>Paper</th><th>Topic</th><th>Note</th></tr></thead>
            <tbody>
              {rows.map((r, i) => (
                <tr key={r.link}>
                  <td><input type="checkbox" checked={r.pick && !r.existing} disabled={!!r.existing || !!running} onChange={(e) => set(i, { pick: e.target.checked })} aria-label={`Add ${r.title}`} /></td>
                  <td>
                    <b>{r.title}</b>
                    <div className="small muted">{r.authors.slice(0, 4).join(", ")}{r.authors.length > 4 ? " et al." : ""}{r.year ? ` · ${r.year}` : ""}{r.venue ? ` · ${r.venue}` : ""} · <a href={r.link} onClick={(e) => { e.preventDefault(); openExternal(r.link); }}>{r.arxiv ?? r.doi}</a></div>
                    {r.existing && <div className="small"><span className="badge ok">already in the list</span> #{r.existing.sno} {r.existing.title}</div>}
                  </td>
                  {r.existing ? <td colSpan={2} className="small muted">not added again</td> : <>
                    <td><input className="search" list="addpaper-topics" value={r.topic} onChange={(e) => set(i, { topic: e.target.value })} placeholder={res?.add_command ? "empty: the library decides" : "topic"} disabled={!!running} aria-label="Topic" style={{ width: "100%", minWidth: "9rem" }} /></td>
                    <td><textarea className="search" value={r.notes} onChange={(e) => set(i, { notes: e.target.value })} disabled={!!running} aria-label="Note" rows={3} style={{ width: "100%", minWidth: "10rem", fontFamily: "inherit" }} /></td>
                  </>}
                </tr>
              ))}
            </tbody>
          </table>
          <div className="controls">
            <button className="btn primary" onClick={add} disabled={busy || !!running || chosen.length === 0}>Add {chosen.length} paper{chosen.length === 1 ? "" : "s"}</button>
            <span className="small muted">{res?.add_command ? <>Written by the library's own tool: <code style={{ wordBreak: "break-all" }}>{res.add_command}</code></> : <>Appended to the end of the list; a backup copy is made first (data folder, <code>papers/backups/</code>).</>}</span>
          </div>
        </>
      )}
      {job && <JobCard j={job} onCancel={() => {}} />}
    </div>
  );
}

function BookBuilder({ papers, sel, defaultTitle, onDone, onClear }: { papers: Paper[]; sel: Set<string>; defaultTitle: string; onDone: () => void; onClear: () => void }) {
  const [title, setTitle] = useState(defaultTitle);
  const [target, setTarget] = useState("new");
  const [colls, setColls] = useState<Collection[]>([]);
  const [job, setJob] = useState<Job | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => { getCollections().then(setColls).catch(() => {}); }, [job?.status]);
  const chosen = papers.filter((p) => sel.has(p.id)).sort((a, b) => a.sno - b.sno);
  const ready = chosen.filter((p) => p.held || p.inbox);
  const fetchable = chosen.filter((p) => !(p.held || p.inbox) && p.downloadable);
  const cannot = chosen.filter((p) => !(p.held || p.inbox) && !p.downloadable);
  const existing = colls.find((c) => c.id === target);
  const already = existing ? chosen.filter((p) => existing.papers.some((x) => x.paper_id === p.id)).length : 0;
  const running = job && (job.status === "running" || job.status === "queued");
  useEffect(() => {
    if (!job || !(job.status === "running" || job.status === "queued")) return;
    const t = setInterval(() => getJobs().then((l) => { const j = l.find((x) => x.id === job.id); if (j) { setJob(j); if (j.status !== "running" && j.status !== "queued") onDone(); } }).catch(() => {}), 1000);
    return () => clearInterval(t);
  }, [job?.id, job?.status]); // eslint-disable-line react-hooks/exhaustive-deps
  const go = async () => {
    setErr(null); setBusy(true);
    try {
      const ids = chosen.map((p) => p.id);
      const r = target === "new" ? await createCollection(title, ids) : await addToCollection(target, ids);
      const l = await getJobs();
      setJob(l.find((x) => x.id === r.job) ?? null);
    } catch (e) { setErr(String((e as Error).message ?? e)); } finally { setBusy(false); }
  };
  const bookId = job?.course.startsWith("collection:") ? job.course.slice(11) : "";
  return (
    <div className="mcourse">
      <b>Make a book from the {chosen.length} selected paper{chosen.length === 1 ? "" : "s"}</b>
      <p className="small muted" style={{ margin: ".3rem 0" }}>
        {ready.length} already on this Mac{fetchable.length > 0 && <>, <b>{fetchable.length} will be downloaded now</b> into the inbox</>}{cannot.length > 0 && <>, <b style={{ color: "var(--bad)" }}>{cannot.length} cannot be fetched automatically</b> (not an arXiv or direct PDF link; they are left out)</>}.
        They are joined in database order into one PDF you can read and mark up.
      </p>
      <div className="controls">
        <select value={target} onChange={(e) => setTarget(e.target.value)} aria-label="New book or existing one" disabled={!!running}>
          <option value="new">A new book</option>
          {colls.map((c) => <option key={c.id} value={c.id}>Add to: {c.title}</option>)}
        </select>
        {target === "new" && <input className="search" value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Title of the book (you choose)" aria-label="Book title" disabled={!!running} />}
        <button className="btn primary" onClick={go} disabled={busy || !!running || (target === "new" && !title.trim()) || (target !== "new" && already === chosen.length)}>
          {target === "new" ? "Create book" : `Add ${chosen.length - already} to the book`}{fetchable.length > 0 ? ` and download ${fetchable.length}` : ""}
        </button>
        <button className="btn sm" onClick={onClear} disabled={!!running}>Cancel</button>
      </div>
      {existing && already > 0 && <div className="small muted">{already} of these are already in “{existing.title}” and will be skipped. New papers go at the end, so your marks stay where they are.</div>}
      {err && <div className="err">{err}</div>}
      {job && (
        <div className={`job ${job.status}`}>
          <div className="jobhead"><b>{job.title}</b><span className={`badge ${job.status === "done" ? "ok" : job.status === "failed" ? "bad" : "info"}`}>{job.status}</span></div>
          <div className="bar resume"><i style={{ width: `${job.total ? (job.done / job.total) * 100 : 0}%` }} /></div>
          <div className="muted small">{job.done} of {job.total} papers{job.status === "running" && job.current_title ? ` · ${job.current_title}` : ""}</div>
          {job.warnings.length > 0 && <div className="small" style={{ color: "var(--warn)" }}>Left out: {job.warnings.join("; ")}</div>}
          {job.message && <div className="err">{job.message}</div>}
          {job.status === "done" && bookId && <div style={{ marginTop: ".4rem" }}><a className="btn sm primary" href={`#/pdf/collection/${bookId}`}>Open the book</a> <a className="btn sm" href="#/paperbooks">Paper Books tab</a></div>}
        </div>
      )}
    </div>
  );
}

function PaperRow({ p, checked, onCheck, expanded, toggle, onChanged }: { p: Paper; checked: boolean; onCheck: () => void; expanded: boolean; toggle: () => void; onChanged: () => void }) {
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const download = async () => {
    setBusy(true);
    setMsg(null);
    try {
      await downloadPaper(p.id);
      onChanged();
    } catch (e) {
      setMsg(String((e as Error).message ?? e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <tr className="row">
        <td><input type="checkbox" checked={checked} onChange={onCheck} aria-label={`Select ${p.title}`} style={{ width: "1.3rem", height: "1.3rem" }} /></td>
        <td className="num">{p.sno}</td>
        <td className="t">
          <button className="linklike" onClick={toggle} aria-expanded={expanded}>{p.title}</button>
          <div className="muted small">{p.topic}{p.est_hours ? ` · ${p.est_hours} h` : ""}{p.date_read ? ` · read ${p.date_read}` : ""}</div>
        </td>
        <td>
          <span className="badges">
            {p.printed && <span className="badge ok">printed</span>}
            {p.held ? <span className="badge ok">PDF</span> : p.missing_local ? <span className="badge bad" title="The row names a PDF that is not at that path">PDF missing</span> : <span className="badge">link only</span>}
            {p.inbox && <span className="badge info" title={`${p.inbox.file}${p.inbox.pages ? `, ${p.inbox.pages} pages` : ""}`}>in inbox</span>}
            {p.banks.length > 0 && <span className="badge info">{p.banks.length > 1 ? `${p.banks.length} banks` : "bank"}</span>}
          </span>
          {msg && <div className="small" style={{ color: "var(--bad)", marginTop: ".25rem" }}>{msg}</div>}
        </td>
        <td>
          <span className="acts">
            {p.held && <a className="btn sm" href={`#/pdf/paper/${p.id}`}>Read</a>}
            {!p.held && p.inbox && <a className="btn sm" href={`#/pdf/inbox/${p.id}`}>Read</a>}
            {!p.held && !p.inbox && p.downloadable && <button className="btn sm primary" onClick={download} disabled={busy} title="Save the PDF into the inbox folder (not into the library)">{busy ? "Downloading…" : "Download"}</button>}
            {p.banks.map((b) => <a key={b.pdf} className="btn sm" href={`#/pdf/bank/${encodeURIComponent(b.folder)}/${encodeURIComponent(b.pdf)}`} title={b.pdf}>Questions</a>)}
            <a className="btn sm" href={p.link} onClick={(e) => { e.preventDefault(); openExternal(p.link); }} title={p.link}>Source ↗</a>
          </span>
        </td>
      </tr>
      {expanded && (
        <tr className="detail"><td /><td /><td colSpan={3}>
          <dl>
            {p.stage && <><dt>Stage</dt><dd>{p.stage}{p.role ? ` · ${p.role}` : ""}</dd></>}
            {p.projects && <><dt>Projects</dt><dd>{p.projects}</dd></>}
            {p.notes && <><dt>Notes</dt><dd>{p.notes}</dd></>}
            <dt>Link</dt><dd><a href={p.link} onClick={(e) => { e.preventDefault(); openExternal(p.link); }}>{p.link}</a></dd>
            {p.resources && <><dt>Resources</dt><dd>{p.resources}</dd></>}
            {!p.held && p.inbox && <><dt>Inbox</dt><dd>{p.inbox.file} · {(p.inbox.bytes / 1e6).toFixed(1)} MB{p.inbox.pages ? ` · ${p.inbox.pages} pages` : ""} · <button className="linklike" onClick={() => openInbox(p.id, true)}>Show in Finder</button> · not in the library yet</dd></>}
            {p.held && <><dt>File</dt><dd><button className="linklike" onClick={() => openPaper(p.id, true)}>Show in Finder</button> · <button className="linklike" onClick={() => openPaper(p.id, false)}>Open in Preview</button></dd></>}
          </dl>
        </td></tr>
      )}
    </>
  );
}

function PdfPage({ kind, refId }: { kind: "paper" | "bank" | "inbox" | "book" | "collection"; refId: string }) {
  const [folder, pdf] = kind === "bank" ? refId.split("/") : ["", ""];
  const [coll, setColl] = useState<Collection | null>(null);
  useEffect(() => { if (kind === "collection") getCollections().then((l) => setColl(l.find((c) => c.id === refId) ?? null)).catch(() => {}); }, [kind, refId]);
  const src = kind === "paper" ? paperPdfUrl(refId) : kind === "inbox" ? inboxPdfUrl(refId) : kind === "book" ? bookPdfUrl(refId) : kind === "collection" ? collectionPdfUrl(refId) : bankPdfUrl(folder, pdf);
  const reveal = (r: boolean) => (kind === "bank" ? openBank(folder, pdf, r) : kind === "inbox" ? openInbox(refId, r) : kind === "book" ? openBook(refId, r) : kind === "collection" ? openCollection(refId, r) : openPaper(refId, r));
  const name = kind === "bank" ? pdf.replace(/\.pdf$/i, "") : kind === "collection" ? (coll?.title ?? "book") : refId.slice(0, 40);
  const chapters = coll ? coll.papers.map((p) => ({ title: p.title, page: p.start })) : undefined;
  return (
    <>
      <div className="crumbs">
        <a href={kind === "book" ? "#/books" : kind === "collection" ? "#/paperbooks" : "#/papers"}>{kind === "book" ? "Books" : kind === "collection" ? "Paper Books" : "Papers"}</a> / {kind === "paper" ? "paper" : kind === "inbox" ? "downloaded paper (inbox)" : kind === "book" ? "book or PDF" : kind === "collection" ? (coll?.title ?? "book") : `question bank · ${folder}`}
        {" "}<button className="btn sm" onClick={() => history.back()}>◀ Back</button>
      </div>
      <PdfAnnotator kind={kind} refId={refId} src={src} name={name} onReveal={reveal} chapters={chapters} />
    </>
  );
}

function BooksPage({ mode }: { mode: "books" | "paperbooks" }) {
  const pb = mode === "paperbooks";
  const [b, setB] = useState<BooksOut | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [q, setQ] = useState("");
  const [folder, setFolder] = useState("");
  const [onlyMarked, setOnlyMarked] = useState(false);
  const [renameId, setRenameId] = useState<string | null>(null);
  const [renameText, setRenameText] = useState("");
  const [confirmDel, setConfirmDel] = useState<string | null>(null);
  const [orderId, setOrderId] = useState<string | null>(null);
  const [orderList, setOrderList] = useState<{ paper_id: string; title: string }[]>([]);
  const terms = q.toLowerCase().split(/\s+/).filter(Boolean);
  const reloadBooks = () => getBooks().then(setB).catch((e) => setErr(String(e.message ?? e)));
  const colls = useMemo(() => (b?.collections ?? []).filter((c) => pb && (!onlyMarked || c.bookmarked) && terms.every((t) => (c.title + " " + c.papers.map((p) => p.title).join(" ")).toLowerCase().includes(t))), [b, folder, q, onlyMarked]); // eslint-disable-line react-hooks/exhaustive-deps
  const toggleCollMark = (id: string, on: boolean) => {
    setB((cur) => cur && { ...cur, collections: cur.collections.map((x) => (x.id === id ? { ...x, bookmarked: on } : x)) });
    setBookmark(id, on).catch((e) => setErr(`Could not save the bookmark: ${String(e.message ?? e)}`));
  };
  const toggleMark = (id: string, on: boolean) => {
    setB((cur) => cur && { ...cur, books: cur.books.map((x) => (x.id === id ? { ...x, bookmarked: on } : x)) });
    setBookmark(id, on).catch((e) => {
      setErr(`Could not save the bookmark: ${String(e.message ?? e)}`);
      setB((cur) => cur && { ...cur, books: cur.books.map((x) => (x.id === id ? { ...x, bookmarked: !on } : x)) });
    });
  };
  useEffect(() => { getBooks().then(setB).catch((e) => setErr(String(e.message ?? e))); }, []);
  const list = useMemo(() => (b?.books ?? []).filter((x) => !pb && (!folder || x.folder === folder) && (!onlyMarked || x.bookmarked) && terms.every((t) => x.rel.toLowerCase().includes(t))), [b, folder, q, onlyMarked]); // eslint-disable-line react-hooks/exhaustive-deps
  const shown = [...list].sort((a, b2) => Number(b2.bookmarked) - Number(a.bookmarked)).slice(0, 400);
  return (
    <>
      <h2>{pb ? "Paper Books" : "Books and other PDFs"}</h2>
      {pb ? <p className="muted small">Books you made by joining papers (from the Papers tab, or built for a reading plan). Open one to read it as a single book; your marks are saved on this Mac. Use <b>Order</b> to rearrange or remove papers.</p> : <p className="muted small">Every PDF under the folders named in <code>[pdf_sources]</code> in <code>config.toml</code>. Open one to read it and mark it up with the pen, highlighter, shapes, text and pictures. Your marks are saved on this Mac, apart from the PDF, which is never changed.</p>}
      {err && <div className="err">{err}</div>}
      {!b && !err && <p className="muted">Looking…</p>}
      {b && (
        <>
          <div className="controls">
            <input className="search" value={q} placeholder="Search by name or folder" onChange={(e) => setQ(e.target.value)} aria-label="Search" />
            <button className={"btn sm" + (onlyMarked ? " primary" : "")} onClick={() => setOnlyMarked(!onlyMarked)} title="Show only the books I bookmarked">★ Bookmarked ({pb ? (b.collections ?? []).filter((x) => x.bookmarked).length : b.books.filter((x) => x.bookmarked).length})</button>
            {!pb && <button className={"btn sm" + (folder === "" ? " primary" : "")} onClick={() => setFolder("")}>All ({b.books.length})</button>}
            {!pb && b.folders.map((f) => (
              <button key={f.path} className={"btn sm" + (folder === f.path ? " primary" : "")} disabled={!f.connected} onClick={() => setFolder(f.path)} title={f.path}>
                {f.path.split("/").filter(Boolean).pop()} ({f.connected ? f.count : "not connected"})
              </button>
            ))}
          </div>
          {!pb && b.folders.length === 0 && <div className="empty">No folders yet. Add a <code>[pdf_sources]</code> section to <code>config.toml</code>.</div>}
          <div className="blist">
            {colls.map((c) => (
              <Fragment key={c.id}>
              <a className="brow" href={`#/pdf/collection/${c.id}`}>
                <span className="bname">
                  <button className="btn sm" style={{ marginRight: 10, minWidth: 44, fontSize: "1.2em", lineHeight: 1 }} aria-pressed={!!c.bookmarked} aria-label={c.bookmarked ? `Remove bookmark from ${c.title}` : `Bookmark ${c.title}`}
                    onClick={(e) => { e.preventDefault(); e.stopPropagation(); toggleCollMark(c.id, !c.bookmarked); }}>{c.bookmarked ? "★" : "☆"}</button>
                  {renameId === c.id ? (
                    <input value={renameText} autoFocus aria-label="Book title" style={{ width: "60%" }}
                      onClick={(e) => { e.preventDefault(); e.stopPropagation(); }}
                      onChange={(e) => setRenameText(e.target.value)}
                      onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); renameCollection(c.id, renameText).then(() => { setRenameId(null); reloadBooks(); }).catch((er) => setErr(String(er.message ?? er))); } if (e.key === "Escape") setRenameId(null); }} />
                  ) : c.title}
                </span>
                <span className="muted small">Book · {c.papers.length} paper{c.papers.length === 1 ? "" : "s"}, {c.papers.reduce((n, p) => n + p.pages, 0)} pages</span>
                <span className="small" style={{ display: "flex", gap: ".4rem" }}>
                  {renameId === c.id ? (
                    <button className="btn sm" onClick={(e) => { e.preventDefault(); e.stopPropagation(); renameCollection(c.id, renameText).then(() => { setRenameId(null); reloadBooks(); }).catch((er) => setErr(String(er.message ?? er))); }}>Save</button>
                  ) : (
                    <button className="btn sm" title="Change the title" onClick={(e) => { e.preventDefault(); e.stopPropagation(); setRenameId(c.id); setRenameText(c.title); }}>Rename</button>
                  )}
                  {confirmDel === c.id ? (
                    <button className="btn sm" style={{ borderColor: "var(--bad)", color: "var(--bad)" }} title="Removes the book only. The papers and your marks stay."
                      onClick={(e) => { e.preventDefault(); e.stopPropagation(); setConfirmDel(null); deleteCollection(c.id).then(reloadBooks).catch((er) => setErr(String(er.message ?? er))); }}>Really delete the book?</button>
                  ) : (
                    <button className="btn sm" onClick={(e) => { e.preventDefault(); e.stopPropagation(); setConfirmDel(c.id); }}>Delete</button>
                  )}
                  <button className="btn sm" title="Change the reading order, or take papers out" onClick={(e) => { e.preventDefault(); e.stopPropagation(); if (orderId === c.id) setOrderId(null); else { setOrderId(c.id); setOrderList(c.papers.map((p) => ({ paper_id: p.paper_id, title: p.title }))); } }}>Order</button>
                </span>
              </a>
              {orderId === c.id && (
                <div className="brow" style={{ display: "block" }}>
                  <p className="muted small">Reading order of “{c.title}”. Your marks and bookmarks move with their paper. Taking a paper out also removes its marks (a copy of the old marks file is kept).</p>
                  <ol style={{ paddingLeft: "1.5rem" }}>
                    {orderList.map((p, i) => (
                      <li key={p.paper_id} style={{ margin: ".25rem 0" }}>
                        <button className="btn sm" disabled={i === 0} aria-label={`Move up: ${p.title}`} onClick={() => setOrderList((l) => { const n = [...l]; [n[i - 1], n[i]] = [n[i], n[i - 1]]; return n; })}>↑</button>{" "}
                        <button className="btn sm" disabled={i === orderList.length - 1} aria-label={`Move down: ${p.title}`} onClick={() => setOrderList((l) => { const n = [...l]; [n[i + 1], n[i]] = [n[i], n[i + 1]]; return n; })}>↓</button>{" "}
                        <button className="btn sm" disabled={orderList.length === 1} aria-label={`Take out: ${p.title}`} title="Take this paper out of the book" onClick={() => setOrderList((l) => l.filter((x) => x.paper_id !== p.paper_id))}>✕</button>{" "}
                        {p.title}
                      </li>
                    ))}
                  </ol>
                  <button className="btn primary sm" onClick={() => reorderCollection(c.id, orderList.map((p) => p.paper_id)).then(() => { setOrderId(null); reloadBooks(); }).catch((er) => setErr(String(er.message ?? er)))}>Save order</button>{" "}
                  <button className="btn sm" onClick={() => setOrderId(null)}>Cancel</button>
                </div>
              )}
              </Fragment>
            ))}
            {shown.map((x) => (
              <a key={x.id} className="brow" href={`#/pdf/book/${x.id}`}>
                <span className="bname">
                  <button
                    className="btn sm"
                    style={{ marginRight: 10, minWidth: 44, fontSize: "1.2em", lineHeight: 1 }}
                    aria-label={x.bookmarked ? `Remove bookmark from ${x.name}` : `Bookmark ${x.name}`}
                    aria-pressed={x.bookmarked}
                    title={x.bookmarked ? "Remove bookmark" : "Bookmark: I am reading this"}
                    onClick={(e) => { e.preventDefault(); e.stopPropagation(); toggleMark(x.id, !x.bookmarked); }}
                  >{x.bookmarked ? "★" : "☆"}</button>
                  {x.name}
                </span>
                <span className="muted small">{x.rel.includes("/") ? x.rel.slice(0, x.rel.lastIndexOf("/")) : ""}</span>
                <span className="muted small">{fmtBytes(x.size)}</span>
              </a>
            ))}
            {!pb && list.length > shown.length && <div className="muted small">Showing the first {shown.length} of {list.length}. Narrow the search to see the rest.</div>}
            {!pb && list.length === 0 && b.folders.length > 0 && <div className="empty">Nothing matches.</div>}
            {pb && colls.length === 0 && <div className="empty">{(b.collections ?? []).length === 0 ? "No paper books yet. Tick papers on the Papers tab and press Make a book." : "Nothing matches."}</div>}
          </div>
        </>
      )}
    </>
  );
}


// ---------- downloads, existing folders, moving ----------

const fmtGB = (n: number | null | undefined) => (n == null ? "–" : n >= 1e11 ? `${(n / 1e9).toFixed(0)} GB` : n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : `${(n / 1e6).toFixed(0)} MB`);

function JobCard({ j, onCancel }: { j: Job; onCancel: (id: number) => void }) {
  const [showLog, setShowLog] = useState(false);
  const active = j.status === "running" || j.status === "queued";
  const overall = j.kind === "move" ? j.percent : j.total > 0 ? ((j.done + (j.status === "running" ? j.percent / 100 : 0)) / j.total) * 100 : 0;
  return (
    <div className={`job ${j.status}`}>
      <div className="jobhead">
        <b>{j.kind === "download" ? "Download" : j.kind === "single" ? "Video" : j.kind === "collection" ? "Book" : j.kind === "addpapers" ? "Papers" : "Move"}: {j.title}</b>
        <span className={`badge ${j.status === "done" ? "ok" : j.status === "failed" ? "bad" : j.status === "cancelled" ? "" : "info"}`}>{j.status}</span>
        <span className="spacer" />
        {active && <button className="btn sm" onClick={() => onCancel(j.id)}>Cancel</button>}
        <button className="btn sm" onClick={() => setShowLog(!showLog)}>{showLog ? "Hide log" : "Log"}</button>
      </div>
      <div className="bar resume" aria-label="progress"><i style={{ width: `${Math.min(100, overall)}%` }} /></div>
      <div className="muted small">
        {j.kind !== "move" ? `${j.done} of ${j.total} ${j.kind === "single" ? "video" : j.kind === "collection" || j.kind === "addpapers" ? "paper" : "lecture"}${j.total === 1 ? "" : "s"} ${j.kind === "addpapers" ? "added" : "verified"}` : `${fmtGB(j.bytes_done)} of ${fmtGB(j.bytes_total)}`}
        {j.status === "running" && j.kind !== "move" && (j.current_index > 0 || j.percent > 0) && <> {j.kind === "single" ? (j.total > 1 ? ` · video ${j.current_index}` : "") : ` · lecture ${j.current_index}`}{j.current_title ? `: ${j.current_title}` : ""} · {j.percent.toFixed(0)}%{j.speed ? ` · ${j.speed}` : ""}{j.eta ? ` · ${j.eta} left` : ""}</>}
        {j.status === "running" && j.kind === "move" && j.current_title && <> · {j.current_title}</>}
      </div>
      {j.message && <div className="small" style={{ color: "var(--bad)" }}>{j.message}</div>}
      {j.failed.length > 0 && <div className="small" style={{ color: "var(--bad)" }}>Not done: {j.failed.join("; ")}</div>}
      {j.warnings.length > 0 && <div className="small muted">Notes: {j.warnings.join("; ")}</div>}
      {showLog && <pre className="log">{j.log.join("\n") || "(no output yet)"}</pre>}
    </div>
  );
}

function FolderPicker({ onPick, onClose, anyFolder = false, noCreate = false, title = "Choose the folder that holds the videos", start }: { onPick: (path: string, create: boolean) => void; onClose: () => void; anyFolder?: boolean; noCreate?: boolean; title?: string; start?: string }) {
  const [b, setB] = useState<BrowseOut | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [typed, setTyped] = useState("");
  const [create, setCreate] = useState(false);
  const go = useCallback((p?: string) => {
    browse(p).then((x) => { setB(x); setTyped(x.path); setErr(null); }).catch((e) => setErr(String(e.message ?? e)));
  }, []);
  useEffect(() => go(start), [go, start]);
  return (
    <div className="panel">
      <div className="controls">
        <b>{title}</b>
        <span className="spacer" />
        <button className="btn sm" onClick={onClose}>Close</button>
      </div>
      <div className="controls">
        <input className="search" value={typed} onChange={(e) => setTyped(e.target.value)} onKeyDown={(e) => e.key === "Enter" && go(typed)} aria-label="Folder path" />
        <button className="btn sm" onClick={() => go(typed)}>Go</button>
        <button className="btn sm" onClick={() => go("/Volumes")}>Disks</button>
        {b?.home && <button className="btn sm" onClick={() => go(b.home!)}>Home</button>}
        {b && b.parent != null && <button className="btn sm" onClick={() => go(b.parent!)}>▲ Up</button>}
      </div>
      {err && <div className="err">{err}</div>}
      {b && (
        <>
          <div className="dirs">
            {b.dirs.length === 0 && <div className="muted small">No sub-folders here.</div>}
            {b.dirs.map((d) => (
              <button key={d.path} className="dirrow" onClick={() => go(d.path)}>
                <span>📁 {d.name}</span>
                <span className="muted small">{d.videos > 0 ? `${d.videos} video${d.videos === 1 ? "" : "s"}` : ""}</span>
              </button>
            ))}
          </div>
          <div className="controls">
            <span className="muted small">{b.path} · {b.videos_here} video{b.videos_here === 1 ? "" : "s"} directly inside</span>
            <span className="spacer" />
            {anyFolder && !noCreate && <label className="small"><input type="checkbox" checked={create} onChange={(e) => setCreate(e.target.checked)} /> create the typed folder if it does not exist</label>}
            {anyFolder && <button className="btn" onClick={() => onPick(typed, create)} title="Use exactly the path in the box above">Use the typed path</button>}
            <button className="btn primary" disabled={!anyFolder && b.videos_here === 0} onClick={() => onPick(b.path, false)} title={!anyFolder && b.videos_here === 0 ? "This folder has no video files directly inside" : ""}>Use this folder</button>
          </div>
        </>
      )}
    </div>
  );
}

function CourseManager({ c, drives, busy, onChanged, onJob }: { c: ManageCourse; drives: DriveOut[]; busy: boolean; onChanged: () => void; onJob: () => void }) {
  const [panel, setPanel] = useState<null | "check" | "point" | "adopt" | "move">(null);
  const [dest, setDest] = useState<string>(""); // "" = where the course is now; else a drive's folder (course goes into <drive>/<course>)
  const [destFolder, setDestFolder] = useState<string>(""); // exactly this folder (wins over dest)
  const [pickDest, setPickDest] = useState(false);
  const [destChosen, setDestChosen] = useState(false); // the download waits until a place is picked
  const [err, setErr] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [plan, setPlan] = useState<DlPlan | null>(null);
  const [checking, setChecking] = useState(false);
  const [force, setForce] = useState(false);
  const [props, setProps] = useState<Proposal[] | null>(null);
  const [picked, setPicked] = useState<Record<string, boolean>>({});
  const [toDrive, setToDrive] = useState<string>("");
  const [keep, setKeep] = useState(false);
  const [mplan, setMplan] = useState<MovePlan | null>(null);
  const [toFolder, setToFolder] = useState<string>(""); // move into exactly this folder
  const [pickMoveFolder, setPickMoveFolder] = useState(false);
  const fail = (e: unknown) => setErr(String((e as Error).message ?? e));
  const reset = () => { setErr(null); setInfo(null); };

  const openCheck = () => { setDest(""); setDestFolder(""); setDestChosen(false); setPickDest(false); check(false, "", ""); };
  const check = async (refresh = false, d = dest, f = destFolder) => {
    reset(); setPanel("check"); setChecking(true); setPlan(null);
    try { setPlan(await planDownload(c.course, refresh, d, f)); } catch (e) { fail(e); } finally { setChecking(false); }
  };
  const chooseDest = (d: string) => { setDest(d); setDestFolder(""); setDestChosen(true); setPickDest(false); check(false, d, ""); };
  const chooseFolder = (f: string) => { setDest(""); setDestFolder(f); setDestChosen(true); setPickDest(false); check(false, "", f); };
  const start = async () => {
    reset();
    try { await startDownload(c.course, force, dest, destFolder); setPanel(null); setPlan(null); onJob(); } catch (e) { fail(e); }
  };
  const point = async (path: string) => {
    reset();
    try { const r = await setLocation(c.course, path); setInfo(`Pointing at ${r.location} (${r.videos} video${r.videos === 1 ? "" : "s"}).`); setPanel(null); onChanged(); } catch (e) { fail(e); }
  };
  const useDefault = async () => { reset(); try { await clearLocation(c.course); onChanged(); } catch (e) { fail(e); } };
  const openAdopt = async () => {
    reset(); setPanel("adopt"); setProps(null);
    try {
      const r = await adoptPreview(c.course);
      setProps(r.proposals);
      setPicked(Object.fromEntries(r.proposals.filter((p) => p.status === "rename").map((p) => [p.file, true])));
    } catch (e) { fail(e); }
  };
  const applyAdopt = async () => {
    reset();
    const renames = (props ?? []).filter((p) => p.status === "rename" && picked[p.file] && p.new_name).map((p) => [p.file, p.new_name!] as [string, string]);
    try { const r = await adoptApply(c.course, renames); setInfo(`Renamed ${r.renamed} file${r.renamed === 1 ? "" : "s"}.`); setPanel(null); onChanged(); } catch (e) { fail(e); }
  };
  const checkMove = async (drive = toDrive, k = keep, folder = toFolder) => {
    reset(); setMplan(null);
    if (!drive && !folder) return;
    try { setMplan(await planMove(c.course, drive, k, c.listed ? undefined : c.drive, folder)); } catch (e) { fail(e); }
  };
  const doMove = async () => {
    reset();
    try { await startMove(c.course, toDrive, keep, c.listed ? undefined : c.drive, toFolder); setPanel(null); setMplan(null); onJob(); } catch (e) { fail(e); }
  };
  // every other drive, including ones that are unplugged (shown but not selectable)
  const targets = drives.filter((d) => !(c.location === d.path + "/" + c.course || c.location.startsWith(d.path + "/" + c.course)));
  return (
    <div className="mcourse">
      <div className="mhead">
        <div>
          <b>{c.title}</b> <span className="tag" title={c.drive === "folder" ? "A folder on no configured drive" : "The drive this course is on now"}>{c.drive === "folder" ? "own folder" : drives.find((d) => d.name === c.drive)?.is_mac ? `${c.drive} (this Mac)` : c.drive}</span>
          {c.listed && c.planned_drive !== c.drive && <span className="tag muted" title="courses.csv names this drive; the course's own folder (locations.json) wins, so downloads and moves start from where it is now">courses.csv: {c.planned_drive}</span>}{!c.listed && <span className="tag" title="Found on a drive but there is no row for it in courses.csv. It can still be moved.">not in CSV</span>}{c.overridden && <span className="tag" title="locations.json points this course at a custom folder">custom folder</span>}
          <div className="muted small">{c.location}{!c.folder_exists && " (not there yet)"}</div>
          <div className="muted small">{c.on_disk} video{c.on_disk === 1 ? "" : "s"} on disk · {fmtGB(c.size_bytes)}{c.listed ? ` · items ${c.items}` : ""}{c.note ? ` · ${c.note}` : ""}</div>
        </div>
        <div className="acts">
          <button className="btn sm primary" onClick={openCheck} disabled={!c.playlist_url} title={c.playlist_url ? "Look up the playlist and see what is missing" : c.listed ? "courses.csv has no playlist_url for this course" : "Add a row with a playlist_url to courses.csv to download more of this course"}>Check &amp; download…</button>
          <button className="btn sm" onClick={() => { reset(); setPanel(panel === "point" ? null : "point"); }} disabled={!c.listed} title={c.listed ? "" : "Needs a row in courses.csv"}>Existing folder…</button>
          <button className="btn sm" onClick={openAdopt} disabled={!c.listed || !c.playlist_url || c.on_disk === 0} title="Number hand-downloaded files by matching their titles to the playlist">Match names…</button>
          <button className="btn sm primary" onClick={() => { reset(); setPanel(panel === "move" ? null : "move"); setMplan(null); setToFolder(""); setToDrive(""); }} disabled={c.on_disk === 0} title="Move or copy this course to the HDD, the SSD, this Mac or a folder you choose">Move…</button>
        </div>
      </div>
      {err && <div className="err">{err}</div>}
      {info && <div className="muted small ok">{info}</div>}

      {panel === "check" && (
        <div className="panel">
          {checking && <div className="muted">Looking up the playlist…</div>}
          {plan && (
            <>
              <div><b>{plan.playlist_title || "Playlist"}</b>: {plan.total} videos in the playlist · wanted {plan.wanted} · already on disk {plan.have} · <b>missing {plan.missing.length}</b></div>
              <div className="muted small">Estimated size about {fmtGB(plan.est_bytes)} · {destChosen ? <>free there {fmtGB(plan.free_bytes)} · saving to {plan.location}</> : "no place chosen yet"}</div>
              <div className="small"><b>Where should it be saved?</b> Pick a place; nothing downloads until you do.</div>
              <div className="controls">
                {[{ key: "", label: c.overridden ? `Where it is now (${c.drive === "folder" ? "its folder" : c.drive})` : `Usual place (${c.drive})`, path: c.location, connected: c.drive_connected, free: c.free_bytes },
                  ...drives.filter((d) => d.name !== c.drive).map((d) => ({ key: d.path, label: d.is_mac ? `This Mac (${d.name})` : d.name, path: `${d.path}/${c.course}`, connected: d.connected, free: d.free_bytes }))].map((o) => {
                  const room = o.free != null && o.free >= plan.est_bytes * 1.03;
                  return (
                    <button key={o.key || "usual"} className={"btn sm" + (destChosen && !destFolder && dest === o.key ? " primary" : "")} disabled={!o.connected}
                      title={o.path || "The drive courses.csv names for this course"}
                      onClick={() => chooseDest(o.key)}>
                      {o.label} · {o.connected ? (o.free != null ? `${fmtGB(o.free)} free${room ? "" : " · too small"}` : "connected") : "not connected"}
                    </button>
                  );
                })}
                <button className={"btn sm" + (destChosen && destFolder ? " primary" : "")} onClick={() => setPickDest(true)} title="Pick the exact folder for this course's videos">
                  {destChosen && destFolder ? `Folder: ${destFolder}` : "Another folder…"}
                </button>
              </div>
              {pickDest && <FolderPicker anyFolder title="Choose the folder for this course's videos (they go straight into it; type a new name to make a new folder)" start={c.location} onPick={(p) => chooseFolder(p)} onClose={() => setPickDest(false)} />}
              {plan.note && <div className="small" style={{ color: "var(--warn)" }}>{plan.note}</div>}
              {destChosen && !plan.drive_connected && <div className="err">That place is not available. Plug the disk in, or pick another place above.</div>}
              {destChosen && plan.drive_connected && !plan.fits && <div className="err">Not enough room (free space or the drive's budget in config.toml).</div>}
              {plan.title_warnings.length > 0 && (
                <div className="err">
                  The playlist's titles do not match your plan for {plan.title_warnings.length} lecture(s); the playlist order may differ:
                  <ul>{plan.title_warnings.slice(0, 4).map((w) => <li key={w} className="small">{w}</li>)}</ul>
                  <label><input type="checkbox" checked={force} onChange={(e) => setForce(e.target.checked)} /> Download anyway</label>
                </div>
              )}
              <div className="controls">
                <button className="btn primary" onClick={start} disabled={busy || !destChosen || plan.missing.length === 0 || !plan.fits || !plan.drive_connected || (plan.title_warnings.length > 0 && !force)}>
                  {!destChosen ? "Pick where to save first" : plan.missing.length === 0 ? "Nothing to download" : `Start download of ${plan.missing.length} lecture${plan.missing.length === 1 ? "" : "s"}`}
                </button>
                <button className="btn sm" onClick={() => check(true)}>Re-read playlist</button>
                <button className="btn sm" onClick={() => setPanel(null)}>Close</button>
              </div>
              <div className="muted small">Starting the download is your go-ahead: it runs yt-dlp at the best resolution available, one lecture at a time, and checks each file when it finishes.</div>
            </>
          )}
        </div>
      )}

      {panel === "point" && (
        <>
          <div className="panel">
            <div className="muted small">Already downloaded this course somewhere? Point the app at that folder. Nothing is copied or changed; courses.csv is untouched.</div>
            <div className="controls">
              <button className="btn primary" onClick={() => setPanel("point")}>Choose folder below…</button>
              {c.overridden && <button className="btn sm" onClick={useDefault}>Use the default place ({c.drive}/{c.course}) instead</button>}
            </div>
          </div>
          <FolderPicker onPick={(p) => point(p)} onClose={() => setPanel(null)} />
        </>
      )}

      {panel === "adopt" && (
        <div className="panel">
          {!props && !err && <div className="muted">Matching file names to the playlist…</div>}
          {props && (
            <>
              <div className="muted small">Each file is matched to a playlist position by its title. Nothing is renamed until you press Apply. Videos that do not match stay as they are.</div>
              <table className="lec">
                <thead><tr><th /><th>File</th><th>Becomes</th><th>Matched to</th></tr></thead>
                <tbody>
                  {props.map((p) => (
                    <tr key={p.file}>
                      <td>{p.status === "rename" && <input type="checkbox" checked={!!picked[p.file]} onChange={(e) => setPicked({ ...picked, [p.file]: e.target.checked })} />}</td>
                      <td className="small">{p.file}</td>
                      <td className="small">{p.status === "rename" ? p.new_name : p.status === "ok" ? <span className="muted">already numbered</span> : <span className="muted">no match, left alone</span>}</td>
                      <td className="small muted">{p.entry_title ? `#${p.index} ${p.entry_title}` : ""}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <div className="controls">
                <button className="btn primary" onClick={applyAdopt} disabled={!props.some((p) => p.status === "rename" && picked[p.file])}>Apply {props.filter((p) => p.status === "rename" && picked[p.file]).length} rename(s)</button>
                <button className="btn sm" onClick={() => setPanel(null)}>Close</button>
              </div>
            </>
          )}
        </div>
      )}

      {panel === "move" && (
        <div className="panel">
          <div className="muted small">Moves this course's files to <code>&lt;drive&gt;/{c.course}</code> on the disk you pick (the HDD, the SSD or this Mac), or into a folder you choose, and carries your ticks and watch positions with them. Files are copied and checked before anything is removed. Unplugged drives are listed but cannot be chosen.</div>
          <div className="controls">
            <label>To drive{" "}
              <select value={toFolder ? "" : toDrive} onChange={(e) => { setToFolder(""); setToDrive(e.target.value); checkMove(e.target.value, keep, ""); }}>
                <option value="">choose…</option>
                {targets.map((d) => <option key={d.name} value={d.name} disabled={!d.connected}>{driveLabel(d)}</option>)}
              </select>
            </label>
            {c.listed && <button className={"btn sm" + (toFolder ? " primary" : "")} onClick={() => setPickMoveFolder(true)}>{toFolder ? `Folder: ${toFolder}` : "Another folder…"}</button>}
            <label><input type="checkbox" checked={keep} onChange={(e) => { setKeep(e.target.checked); checkMove(toDrive, e.target.checked); }} /> Keep the original (copy instead of move)</label>
          </div>
          {pickMoveFolder && <FolderPicker anyFolder title="Choose the folder to move this course into (an empty or new folder; the videos go straight into it)" onPick={(p) => { setPickMoveFolder(false); setToFolder(p); setToDrive(""); checkMove("", keep, p); }} onClose={() => setPickMoveFolder(false)} />}
          {mplan && (
            <>
              <div className="small">{mplan.files} file{mplan.files === 1 ? "" : "s"}, {fmtGB(mplan.total_bytes)} · {mplan.same_fs && !keep ? "same disk: an instant move" : "different disk: a copy, then the originals are removed after checking"}{mplan.already_there > 0 ? ` · ${mplan.already_there} already there` : ""}</div>
              <div className="muted small">{mplan.from} → {mplan.to} · free at destination {fmtGB(mplan.free_bytes)}</div>
              {!mplan.ok && <div className="err">{mplan.reason}</div>}
              <div className="controls">
                <button className="btn primary" onClick={doMove} disabled={busy || !mplan.ok}>{keep ? "Copy" : "Move"} {fmtGB(mplan.total_bytes)}</button>
                <button className="btn sm" onClick={() => setPanel(null)}>Close</button>
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}

const driveLabel = (d: DriveOut) => `${d.name}${d.is_mac ? " (this Mac)" : ""} — ${d.connected ? `${fmtGB(d.free_bytes)} free` : "not connected"}`;

function DrivesPanel({ drives, onChanged }: { drives: DriveOut[]; onChanged: () => void }) {
  const [editing, setEditing] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const apply = async (name: string, path: string, create: boolean) => {
    setErr(null); setMsg(null);
    try { const r = await setDrive(name, path, create); setMsg(`${name} now points at ${r.path}`); setEditing(null); onChanged(); } catch (e) { setErr(String((e as Error).message ?? e)); }
  };
  return (
    <>
      <h2>Drives</h2>
      <p className="muted small">Courses live in a folder on a drive. If a disk has a different name than the one in <code>config.toml</code> (for example a new SSD), point the drive at its folder here.</p>
      {err && <div className="err">{err}</div>}
      {msg && <div className="muted small ok">{msg}</div>}
      <div className="drivegrid">
        {drives.map((d) => (
          <div key={d.name} className={`drivecard ${d.connected ? "" : "off"}`}>
            <div><b>{d.name}</b> <span className="tag">{d.is_mac ? "this Mac" : "external"}</span> <span className={`badge ${d.connected ? "ok" : "bad"}`}>{d.connected ? "connected" : "not connected"}</span></div>
            <div className="muted small">{d.path}</div>
            <div className="muted small">{d.connected ? `${fmtGB(d.free_bytes)} free of ${fmtGB(d.total_bytes)}` : "plug the disk in, or point this drive at another folder"}{d.budget_gb ? ` · budget ${d.budget_gb} GB` : ""}</div>
            <button className="btn sm" onClick={() => setEditing(editing === d.name ? null : d.name)}>{editing === d.name ? "Cancel" : "Change folder…"}</button>
          </div>
        ))}
      </div>
      {editing && <FolderPicker anyFolder title={`Choose the folder for the "${editing}" drive`} onPick={(p, create) => apply(editing, p, create)} onClose={() => setEditing(null)} />}
    </>
  );
}

function SinglesPanel({ jobs, onJob }: { jobs: Job[]; onJob: () => void }) {
  const [items, setItems] = useState<Single[]>([]);
  const [lastDest, setLastDest] = useState<string | null>(null);
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [picking, setPicking] = useState<string | null>(null);
  const load = useCallback(() => getSingles().then((x) => { setItems(x.items); setLastDest(x.last_dest); }).catch((e) => setErr(String(e.message ?? e))), []);
  useEffect(() => { load(); }, [load]);
  const active = jobs.some((j) => j.kind === "single" && (j.status === "running" || j.status === "queued"));
  const was = useRef(false);
  useEffect(() => { if (was.current && !active) load(); was.current = active; }, [active, load]);
  const running = (id: string) => jobs.some((j) => j.course === `single:${id}` && (j.status === "running" || j.status === "queued"));
  const add = async () => {
    if (!url.trim()) return;
    setBusy(true); setErr(null);
    try { await addSingle(url.trim()); setUrl(""); await load(); } catch (e) { setErr(String((e as Error).message ?? e)); } finally { setBusy(false); }
  };
  const pick = async (id: string, path: string, create: boolean) => {
    setPicking(null); setErr(null);
    try { await downloadSingle(id, path, create); onJob(); } catch (e) { setErr(String((e as Error).message ?? e)); }
  };
  return (
    <>
      <h2>Single videos</h2>
      <p className="muted small">Paste a YouTube video link (for example from freeCodeCamp) and press Add: the link is saved here. Nothing downloads until you press <b>Download…</b> and choose the folder to save it in.</p>
      <div className="controls">
        <input className="search" placeholder="https://www.youtube.com/watch?v=…" value={url} onChange={(e) => setUrl(e.target.value)} onKeyDown={(e) => e.key === "Enter" && add()} aria-label="YouTube link" />
        <button className="btn primary" disabled={busy || !url.trim()} onClick={add}>{busy ? "Looking up…" : "Add"}</button>
      </div>
      {err && <div className="err">{err}</div>}
      {items.length === 0 && <div className="empty">No saved links yet.</div>}
      {items.map((it) => (
        <div key={it.id} className="panel">
          <div className="controls">
            <b>{it.title}</b>
            <span className={`badge ${it.status === "downloaded" ? "ok" : "info"}`}>{it.status === "downloaded" ? "downloaded" : "saved"}</span>
            <span className="spacer" />
            <button className="btn primary sm" disabled={running(it.id)} onClick={() => setPicking(it.id)}>{it.status === "downloaded" ? "Download again…" : "Download…"}</button>
            {it.status === "downloaded" && <button className="btn sm" onClick={() => revealSingle(it.id)}>Show in Finder</button>}
            <button className="btn sm" disabled={running(it.id)} onClick={() => removeSingle(it.id).then(load).catch((e) => setErr(String(e.message ?? e)))} title="Forgets the link only; a downloaded file is never deleted">Remove link</button>
          </div>
          <div className="muted small">
            {it.uploader && <>{it.uploader} · </>}{it.kind === "playlist" ? `playlist of ${it.count} videos` : "video"}{it.duration ? ` · ${fmtTime(it.duration)}` : ""} · <a href={it.url} onClick={(e) => { e.preventDefault(); openExternal(it.url); }}>{it.url}</a>
          </div>
          {it.status === "downloaded" && it.dest && <div className="small">Saved in <code>{it.dest}</code></div>}
          {picking === it.id && <FolderPicker title="Where should this be saved?" anyFolder start={it.dest ?? lastDest ?? undefined} onClose={() => setPicking(null)} onPick={(p, c) => pick(it.id, p, c)} />}
        </div>
      ))}
    </>
  );
}

function ManagePage({ onChanged }: { onChanged: () => void }) {
  const [m, setM] = useState<ManageState | null>(null);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const prevActive = useRef(false);

  const loadManage = useCallback(() => getManage().then((x) => { setM(x); setErr(null); }).catch((e) => setErr(String(e.message ?? e))), []);
  const loadJobs = useCallback(() => getJobs().then(setJobs).catch(() => {}), []);
  useEffect(() => { loadManage(); loadJobs(); }, [loadManage, loadJobs]);

  const active = jobs.some((j) => j.status === "running" || j.status === "queued");
  useEffect(() => {
    const t = setInterval(loadJobs, active ? 1000 : 4000);
    return () => clearInterval(t);
  }, [active, loadJobs]);
  // when a job finishes, re-read the courses (new files, new location) and the library
  useEffect(() => {
    if (prevActive.current && !active) { loadManage(); onChanged(); }
    prevActive.current = active;
  }, [active, loadManage, onChanged]);

  if (err) return <div className="banner">{err}</div>;
  if (!m) return <p className="muted">Loading…</p>;
  if (m.error) return <div className="banner">{m.error}</div>;
  const busyFor = (course: string) => jobs.some((j) => j.course === course && (j.status === "running" || j.status === "queued"));
  const refreshAll = () => { loadManage(); onChanged(); };

  return (
    <>
      <h1>Downloads</h1>
      <p className="muted">
        {m.ytdlp ? <>yt-dlp {m.ytdlp}</> : <span style={{ color: "var(--bad)" }}>yt-dlp was not found on this Mac, so downloading is unavailable (watching and moving still work).</span>}
      </p>
      {m.home && <p className="muted small">Your data folder (config.toml, courses.csv, annotations, books): <code>{m.home}</code></p>}

      <DrivesPanel drives={m.drives} onChanged={refreshAll} />

      <h2>Jobs</h2>
      {jobs.length === 0 ? <div className="empty">No downloads or moves yet. Use “Check &amp; download…” on a course below, or add a video link.</div> : jobs.slice(0, 8).map((j) => <JobCard key={j.id} j={j} onCancel={(id) => cancelJob(id).then(loadJobs).catch(() => {})} />)}

      <SinglesPanel jobs={jobs} onJob={loadJobs} />

      <h2>Courses</h2>
      <p className="muted small">Every course in <code>courses.csv</code> plus any folder of videos found on a connected drive. Use <b>Move…</b> to send a course to the HDD, the SSD or this Mac. Edit <code>courses.csv</code> to add a course, then press Refresh. Pressing “Start download” is the go-ahead, even for a course marked <code>enabled=no</code>.</p>
      {m.courses.map((c) => <CourseManager key={c.course} c={c} drives={m.drives} busy={busyFor(c.course)} onChanged={refreshAll} onJob={loadJobs} />)}
    </>
  );
}
