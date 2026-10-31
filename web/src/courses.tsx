import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Single, getSingles, revealSingle, Course, MacInfo, macCopy, macRemove, Job, getJobs, Lecture, State, openExternal, fmtBytes, fmtHours, fmtTime, getSubs, mediaUrl, saveProgress, setDone, setSpeed } from "./api";
import { courseHref, courseSeconds, keyOf, playHref } from "./nav";
import { fmtGB } from "./manage";

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

export function Home({ state, onChanged }: { state: State; onChanged: () => void }) {
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

export function Tick({ on, onClick, label }: { on: boolean; onClick: () => void; label: string }) {
  return (
    <button className={`tick ${on ? "on" : ""}`} onClick={onClick} aria-pressed={on} aria-label={label} title={label}>
      {on ? "✓" : ""}
    </button>
  );
}

// ---------- course page ----------

export function CoursePage({ state, drive, course, patch }: { state: State; drive: string; course: string; patch: (d: string, c: string, k: string, p: Partial<Lecture>) => void }) {
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

export function PlayPage({ state, drive, course, file, patch, patchSpeed, autoNext, setAutoNext }: {
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
