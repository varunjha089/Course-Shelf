import { Fragment, useCallback, useEffect, useMemo, useState } from "react";
import { Job, getJobs, Paper, PapersState, bankPdfUrl, bookPdfUrl, BooksOut, getBooks, setBookmark, Collection, getCollections, createCollection, addToCollection, renameCollection, reorderCollection, deleteCollection, collectionPdfUrl, openCollection, openBook, downloadPaper, getPapers, inboxPdfUrl, openBank, openExternal, openInbox, openPaper, paperPdfUrl, fmtBytes, LookedUp, LookupOut, NewPaper, addPapers, lookupPapers } from "./api";
import { PdfAnnotator } from "./pdfview";

import { JobCard } from "./manage";

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

export function PapersPage() {
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

export function PdfPage({ kind, refId }: { kind: "paper" | "bank" | "inbox" | "book" | "collection"; refId: string }) {
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

export function BooksPage({ mode }: { mode: "books" | "paperbooks" }) {
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
