import { useCallback, useEffect, useRef, useState } from "react";
import { TidyOut, tidyFolders, Single, addSingle, downloadSingle, getSingles, removeSingle, revealSingle, BrowseOut, DlPlan, DriveOut, Job, ManageCourse, ManageState, setDrive, MovePlan, Proposal, adoptApply, adoptPreview, browse, cancelJob, clearLocation, getJobs, getManage, planDownload, planMove, setLocation, startDownload, startMove, openExternal, fmtTime } from "./api";

// ---------- downloads, existing folders, moving ----------

export const fmtGB = (n: number | null | undefined) => (n == null ? "–" : n >= 1e11 ? `${(n / 1e9).toFixed(0)} GB` : n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : `${(n / 1e6).toFixed(0)} MB`);

export function JobCard({ j, onCancel }: { j: Job; onCancel: (id: number) => void }) {
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
          <button className="btn sm" onClick={async () => { reset(); try { setInfo(tidyText(await tidyFolders(c.course))); } catch (e) { fail(e); } }} disabled={c.on_disk === 0} title="Shrink the .info.json files and hide the side files, so the folder shows only the videos">Tidy</button>
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

const tidyText = (r: TidyOut) => r.shrunk === 0 && r.hidden === 0 ? "Already tidy." : `Tidied: ${r.shrunk} details file${r.shrunk === 1 ? "" : "s"} shrunk (${fmtGB(r.bytes_saved)} saved), ${r.hidden} side file${r.hidden === 1 ? "" : "s"} hidden.`;

/** One button that tidies every course folder that can be reached (connected drives and custom folders). */
function TidyAll() {
  const [msg, setMsg] = useState<string | null>(null);
  const [running, setRunning] = useState(false);
  const run = async () => {
    setRunning(true); setMsg(null);
    try { setMsg(tidyText(await tidyFolders())); } catch (e) { setMsg(String((e as Error).message ?? e)); } finally { setRunning(false); }
  };
  return (
    <div className="controls">
      <button className="btn sm" onClick={run} disabled={running} title="Shrink each lecture's .info.json and hide the .info.json and .vtt files, so course folders show only the videos. Videos are never touched.">{running ? "Tidying…" : "Tidy all course folders"}</button>
      {msg && <span className="muted small">{msg}</span>}
    </div>
  );
}

export function ManagePage({ onChanged }: { onChanged: () => void }) {
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
      <TidyAll />
      <p className="muted small">Every course in <code>courses.csv</code> plus any folder of videos found on a connected drive. Use <b>Move…</b> to send a course to the HDD, the SSD, this Mac or a folder you choose. Edit <code>courses.csv</code> to add a course, then press Refresh. Pressing “Start download” is the go-ahead, even for a course marked <code>enabled=no</code>.</p>
      {m.courses.map((c) => <CourseManager key={c.course} c={c} drives={m.drives} busy={busyFor(c.course)} onChanged={refreshAll} onJob={loadJobs} />)}
    </>
  );
}
