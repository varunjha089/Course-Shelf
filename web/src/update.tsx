import { useCallback, useEffect, useState } from "react";
import { UpdateCheck, checkUpdate, getJobs, installUpdate, restartForUpdate } from "./api";

// ---------- updates: a banner when a newer CourseShelf is published, install and restart ----------

type Phase = "idle" | "checking" | "available" | "installing" | "ready" | "uptodate" | "failed";

/** Checks once when the app opens (quietly) and from the "Check for updates" button (says "up to date" too). */
export function UpdateBanner() {
  const [phase, setPhase] = useState<Phase>("idle");
  const [check, setCheck] = useState<UpdateCheck | null>(null);
  const [msg, setMsg] = useState<string>("");
  const [notes, setNotes] = useState(false);

  const run = useCallback(async (manual: boolean) => {
    setPhase("checking"); setMsg("");
    try {
      const c = await checkUpdate();
      setCheck(c);
      setPhase(c.release ? "available" : manual ? "uptodate" : "idle");
    } catch (e) {
      setMsg(String((e as Error).message ?? e));
      setPhase(manual ? "failed" : "idle");
    }
  }, []);
  useEffect(() => { run(false); }, [run]);
  useEffect(() => {
    const again = () => run(true);
    addEventListener("courseshelf-check-update", again);
    return () => removeEventListener("courseshelf-check-update", again);
  }, [run]);

  const install = async () => {
    setPhase("installing"); setMsg("");
    try {
      const { job } = await installUpdate();
      for (;;) {
        await new Promise((r) => setTimeout(r, 700));
        const j = (await getJobs()).find((x) => x.id === job);
        if (!j) continue;
        if (j.status === "done") { setMsg(j.log[j.log.length - 1] ?? ""); setPhase("ready"); return; }
        if (j.status === "failed" || j.status === "cancelled") { setMsg(j.message || j.log[j.log.length - 1] || "the update stopped"); setPhase("failed"); return; }
      }
    } catch (e) { setMsg(String((e as Error).message ?? e)); setPhase("failed"); }
  };
  const restart = async () => {
    try { await restartForUpdate(); setMsg("Restarting…"); } catch (e) { setMsg(String((e as Error).message ?? e)); }
  };

  const r = check?.release;
  if (phase === "idle" || phase === "checking" && !r) return phase === "checking" ? <div className="update muted small">Checking for updates…</div> : null;
  return (
    <div className="update" role="status">
      {phase === "uptodate" && <><b>You're up to date.</b> CourseShelf {check?.current} is the newest version. <button className="btn sm" onClick={() => setPhase("idle")}>OK</button></>}
      {phase === "failed" && <><b>Not right now.</b> {msg} <button className="btn sm" onClick={() => setPhase("idle")}>OK</button></>}
      {r && (phase === "available" || phase === "installing") && (
        <>
          <b>CourseShelf {r.version} is available</b> <span className="muted">(you have {check?.current})</span>{" "}
          {r.asset
            ? <button className="btn sm primary" disabled={phase === "installing"} onClick={install}>{phase === "installing" ? "Installing…" : `Install and restart (${(r.asset.size / 1e6).toFixed(0)} MB)`}</button>
            : <a className="btn sm" href={r.page} target="_blank" rel="noreferrer">Open the download page</a>}{" "}
          {r.notes && <button className="btn sm" onClick={() => setNotes(!notes)}>{notes ? "Hide" : "What's new"}</button>}{" "}
          <button className="btn sm" onClick={() => setPhase("idle")} disabled={phase === "installing"}>Later</button>
          {notes && <pre className="notes">{r.notes}</pre>}
        </>
      )}
      {phase === "ready" && <><b>{msg}</b> <button className="btn sm primary" onClick={restart}>Restart now</button> <button className="btn sm" onClick={() => setPhase("idle")}>Later</button></>}
    </div>
  );
}

/** Asks the banner to check now (it says "up to date" when there is nothing newer). */
export const checkForUpdatesNow = () => dispatchEvent(new Event("courseshelf-check-update"));
