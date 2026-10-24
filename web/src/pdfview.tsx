// PDF viewer with annotation: pen, highlighter, eraser, shapes, text, images, notes, bookmarks, search, export.
// The PDF itself is never changed; marks are saved by the server next to the app (see crates/server/src/annot.rs).

import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as pdfjs from "pdfjs-dist";
import type { PDFDocumentProxy } from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";
import { annotExport, annotImageUrl, annotLoad, annotReveal, annotSave, annotUploadImage } from "./api";
import { Doc, El, ImageEl, Pt, Shape, Stroke, TextBox, arrowHead, bbox, emptyDoc, exportAnnotated, hasText, hit, isShape, moveEl, normDoc, penPath, polyline, resizeEl, uid } from "./annotmodel";

pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;

type Tool = "view" | "select" | "pen" | "hl" | "eraser" | "rect" | "ellipse" | "line" | "arrow" | "text" | "image";
type Size = { w: number; h: number };

const COLORS = ["#111111", "#d92d20", "#1a56db", "#16a34a", "#f5c400", "#f97316", "#9333ea", "#ffffff"];
const HL_COLORS = ["#f5c400", "#22c55e", "#f472b6", "#38bdf8", "#f97316"];
const TOOLS: { t: Tool; label: string; key: string; tip: string }[] = [
  { t: "view", label: "Read", key: "G", tip: "Read only: scroll and select text (G)" },
  { t: "select", label: "Select", key: "V", tip: "Click a mark to move, resize, recolour or delete it (V). Double-click text or a shape to edit its words." },
  { t: "pen", label: "Pen", key: "P", tip: "Pen (P)" },
  { t: "hl", label: "Highlighter", key: "H", tip: "Highlighter (H)" },
  { t: "eraser", label: "Eraser", key: "E", tip: "Eraser: removes whole marks you touch (E)" },
  { t: "rect", label: "Box", key: "R", tip: "Rectangle (R). Double-click it with Select to type inside." },
  { t: "ellipse", label: "Oval", key: "O", tip: "Ellipse (O). Double-click it with Select to type inside." },
  { t: "line", label: "Line", key: "L", tip: "Line (L)" },
  { t: "arrow", label: "Arrow", key: "A", tip: "Arrow (A)" },
  { t: "text", label: "Text", key: "T", tip: "Click the page and type (T)" },
];

interface Ctx {
  tool: Tool;
  color: string;
  width: number;
  fill: boolean;
  fs: number;
  zoom: number;
  selectedId: string | null;
  pendingImage: { file: string; w: number; h: number } | null;
  key: string;
  add: (el: El, edit?: boolean) => void;
  replace: (el: El) => void;
  remove: (ids: string[]) => void;
  select: (id: string | null) => void;
  edit: (id: string | null) => void;
  finishEdit: (id: string, text: string, h: number) => void;
  probe: (type: string, pressure: number) => void;
  doneImage: () => void;
}

// ---------- one page: canvas from pdf.js, SVG layer for the marks ----------

const PageView = memo(function PageView({ pdf, n, size, zoom, mg, els, ctx, root, tool, selectedId, editingId }: {
  pdf: PDFDocumentProxy; n: number; size: Size; zoom: number; mg: number; els: El[]; ctx: { current: Ctx }; root: { current: HTMLDivElement | null };
  tool: Tool; selectedId: string | null; editingId: string | null;
}) {
  const wrap = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const svg = useRef<SVGSVGElement>(null);
  const [vis, setVis] = useState(false);
  const [draft, setDraft] = useState<El | null>(null);
  const [gone, setGone] = useState<Set<string>>(new Set());
  const [hover, setHover] = useState<{ x: number; y: number } | null>(null);
  const drag = useRef<null | { mode: "draw" | "erase" | "move" | "resize"; sx: number; sy: number; orig?: El; ids?: Set<string>; moved?: boolean }>(null);
  const draftRef = useRef<El | null>(null);
  const setDraftBoth = (d: El | null) => { draftRef.current = d; setDraft(d); };

  useEffect(() => {
    const el = wrap.current;
    if (!el) return;
    const io = new IntersectionObserver((e) => setVis(e.some((x) => x.isIntersecting)), { root: root.current, rootMargin: "1400px 0px" });
    io.observe(el);
    return () => io.disconnect();
  }, [root]);

  useEffect(() => {
    if (!vis) return;
    let cancelled = false;
    let task: { cancel: () => void; promise: Promise<unknown> } | null = null;
    (async () => {
      const page = await pdf.getPage(n);
      const dpr = window.devicePixelRatio || 1;
      const base = page.getViewport({ scale: 1 });
      const scale = Math.min(zoom * dpr, Math.sqrt(18e6 / (base.width * base.height)));
      const vp = page.getViewport({ scale });
      const c = canvas.current;
      if (!c || cancelled) return;
      c.width = Math.floor(vp.width);
      c.height = Math.floor(vp.height);
      const g = c.getContext("2d");
      if (!g) return;
      task = page.render({ canvas: c, canvasContext: g, viewport: vp });
      await task.promise.catch((e) => { if (!cancelled) console.error("page render failed", n, e); });
    })();
    return () => { cancelled = true; task?.cancel(); };
  }, [vis, pdf, n, zoom]);

  const mx = size.w * mg; // blank margin on each side, in page units
  const pos = (e: React.PointerEvent) => {
    const r = svg.current!.getBoundingClientRect();
    return { x: (e.clientX - r.left) / ctx.current.zoom - mx, y: (e.clientY - r.top) / ctx.current.zoom };
  };
  const topHit = (x: number, y: number, r: number) => {
    for (let i = els.length - 1; i >= 0; i--) if (!gone.has(els[i].id) && hit(els[i], x, y, r)) return els[i];
    return null;
  };

  const down = (e: React.PointerEvent<SVGSVGElement>) => {
    const c = ctx.current;
    if (e.button !== 0 && e.pointerType !== "pen") return;
    if (e.button === 2) return;
    const { x, y } = pos(e);
    const tl: Tool = e.buttons & 32 ? "eraser" : c.tool; // the pen's eraser end, if the driver reports it
    e.currentTarget.setPointerCapture(e.pointerId);
    c.probe(e.pointerType, e.pressure);
    if (tl === "pen" || tl === "hl") {
      const pr = e.pointerType === "pen" || (e.pointerType !== "mouse" && e.pressure > 0 && e.pressure !== 0.5);
      const s: Stroke = { id: uid(), page: n, t: tl, color: c.color, w: c.width, pts: [[x, y, e.pressure || 0.5]], pr: tl === "pen" && pr };
      drag.current = { mode: "draw", sx: x, sy: y };
      setDraftBoth(s);
    } else if (tl === "eraser") {
      const ids = new Set<string>();
      const h = topHit(x, y, 6);
      if (h) ids.add(h.id);
      drag.current = { mode: "erase", sx: x, sy: y, ids };
      setGone(new Set(ids));
    } else if (tl === "rect" || tl === "ellipse" || tl === "line" || tl === "arrow") {
      const s: Shape = { id: uid(), page: n, t: tl, color: c.color, x1: x, y1: y, x2: x, y2: y, w: c.width, fill: c.fill && (tl === "rect" || tl === "ellipse") ? c.color : null, text: "", fs: c.fs };
      drag.current = { mode: "draw", sx: x, sy: y };
      setDraftBoth(s);
    } else if (tl === "text") {
      const t: TextBox = { id: uid(), page: n, t: "text", color: c.color, x, y, w: 220, h: c.fs * 1.5, text: "", fs: c.fs };
      c.add(t, true);
    } else if (tl === "image" && c.pendingImage) {
      const p = c.pendingImage;
      const w = Math.min(320, p.w);
      const im: ImageEl = { id: uid(), page: n, t: "image", color: "#000000", x, y, w, h: (w * p.h) / p.w, file: p.file };
      c.add(im);
      c.doneImage();
    } else if (tl === "select") {
      const sel = els.find((el) => el.id === c.selectedId && el.t !== "pen" && el.t !== "hl");
      if (sel) {
        const b = bbox(sel);
        const hx = b.x + b.w, hy = b.y + b.h;
        if (Math.hypot(x - hx, y - hy) <= 10 / c.zoom) {
          drag.current = { mode: "resize", sx: x, sy: y, orig: sel };
          return;
        }
      }
      const h = topHit(x, y, 5);
      c.select(h ? h.id : null);
      if (h) drag.current = { mode: "move", sx: x, sy: y, orig: h };
    }
  };

  const move = (e: React.PointerEvent<SVGSVGElement>) => {
    const c = ctx.current;
    const { x, y } = pos(e);
    c.probe(e.pointerType, e.pressure);
    const d = drag.current;
    if (!d) {
      if (c.tool === "pen" || c.tool === "hl" || c.tool === "eraser") setHover({ x, y });
      return;
    }
    setHover({ x, y });
    if (d.mode === "draw") {
      const cur = draftRef.current;
      if (!cur) return;
      if (cur.t === "pen" || cur.t === "hl") {
        const evs = (e.nativeEvent as PointerEvent).getCoalescedEvents?.() ?? [];
        const r = svg.current!.getBoundingClientRect();
        const pts: Pt[] = (evs.length ? evs : [e.nativeEvent as PointerEvent]).map((pe) => [(pe.clientX - r.left) / c.zoom - mx, (pe.clientY - r.top) / c.zoom, pe.pressure || 0.5]);
        setDraftBoth({ ...cur, pts: [...cur.pts, ...pts] });
      } else if (isShape(cur)) {
        setDraftBoth({ ...cur, x2: x, y2: y });
      }
    } else if (d.mode === "erase") {
      const h = topHit(x, y, 6);
      if (h && !d.ids!.has(h.id)) {
        d.ids!.add(h.id);
        setGone(new Set(d.ids));
      }
    } else if (d.mode === "move" && d.orig) {
      d.moved = true;
      setDraftBoth(moveEl(d.orig, x - d.sx, y - d.sy));
    } else if (d.mode === "resize" && d.orig) {
      d.moved = true;
      setDraftBoth(resizeEl(d.orig, x, y));
    }
  };

  const up = () => {
    const c = ctx.current;
    const d = drag.current;
    drag.current = null;
    if (!d) return;
    const cur = draftRef.current;
    if (d.mode === "draw" && cur) {
      let keep = true;
      if (isShape(cur)) keep = Math.hypot(cur.x2 - cur.x1, cur.y2 - cur.y1) > 3;
      if (keep) c.add(cur);
    } else if (d.mode === "erase") {
      if (d.ids && d.ids.size) c.remove([...d.ids]);
      setGone(new Set());
    } else if ((d.mode === "move" || d.mode === "resize") && cur && d.moved) {
      c.replace(cur);
    }
    setDraftBoth(null);
  };

  const dbl = (e: React.MouseEvent<SVGSVGElement>) => {
    if (ctx.current.tool !== "select") return;
    const r = svg.current!.getBoundingClientRect();
    const x = (e.clientX - r.left) / ctx.current.zoom - mx, y = (e.clientY - r.top) / ctx.current.zoom;
    const h = topHit(x, y, 5);
    if (h && hasText(h)) ctx.current.edit(h.id);
  };

  const shown = els.filter((e) => !gone.has(e.id) && e.id !== draft?.id);
  const editing = editingId ? els.find((e) => e.id === editingId) : null;
  const sel = (draft && draft.id === selectedId ? draft : els.find((e) => e.id === selectedId)) ?? null;
  const cursor = tool === "pen" || tool === "hl" ? "crosshair" : tool === "eraser" ? "none" : tool === "text" ? "text" : tool === "select" ? "default" : tool === "image" ? "copy" : "crosshair";
  const brush = tool === "pen" || tool === "hl" ? ctx.current.width / 2 : tool === "eraser" ? 6 : 0;

  return (
    <div ref={wrap} className="pgwrap" data-page={n} style={{ width: (size.w + 2 * mx) * zoom, height: size.h * zoom }}>
      {vis && <canvas ref={canvas} className="pgcanvas" style={{ left: mx * zoom, width: size.w * zoom, height: size.h * zoom, outline: mx > 0 ? "1px solid #d4d4d0" : undefined }} />}
      {vis && (
        <svg
          ref={svg}
          className="pgsvg"
          viewBox={`${-mx} 0 ${size.w + 2 * mx} ${size.h}`}
          style={{ width: (size.w + 2 * mx) * zoom, height: size.h * zoom, pointerEvents: tool === "view" ? "none" : "auto", cursor }}
          onPointerDown={down}
          onPointerMove={move}
          onPointerUp={up}
          onPointerCancel={up}
          onPointerLeave={() => !drag.current && setHover(null)}
          onDoubleClick={dbl}
        >
          {shown.map((el) => <g key={el.id} data-el={el.id}><MarkView el={el} zoom={zoom} ctxKey={ctx.current.key} editing={el.id === editingId} /></g>)}
          {draft && <g data-el={draft.id}><MarkView el={draft} zoom={zoom} ctxKey={ctx.current.key} editing={false} /></g>}
          {sel && sel.page === n && <Handles el={sel} zoom={zoom} />}
          {hover && brush > 0 && <circle cx={hover.x} cy={hover.y} r={brush} fill="none" stroke={tool === "eraser" ? "#888" : ctx.current.color} strokeWidth={0.6 / Math.max(zoom, 0.3)} style={{ pointerEvents: "none" }} />}
        </svg>
      )}
      {vis && editing && <EditBox el={editing} zoom={zoom} mx={mx} ctx={ctx} />}
      <span className="pgnum">{n}</span>
    </div>
  );
}, (a, b) =>
  a.pdf === b.pdf && a.n === b.n && a.zoom === b.zoom && a.mg === b.mg && a.tool === b.tool && a.selectedId === b.selectedId && a.editingId === b.editingId &&
  a.size.w === b.size.w && a.size.h === b.size.h && a.els === b.els,
);

function MarkView({ el, zoom, ctxKey, editing }: { el: El; zoom: number; ctxKey: string; editing: boolean }) {
  void zoom;
  switch (el.t) {
    case "pen":
      return <path d={penPath(el.pts, el.w, el.pr)} fill={el.color} />;
    case "hl":
      return <path d={polyline(el.pts)} fill="none" stroke={el.color} strokeWidth={el.w} strokeOpacity={0.35} strokeLinecap="round" strokeLinejoin="round" style={{ mixBlendMode: "multiply" }} />;
    case "image":
      return <image href={annotImageUrl(ctxKey, el.file)} x={el.x} y={el.y} width={el.w} height={el.h} preserveAspectRatio="none" />;
    case "text":
      return editing ? null : (
        <foreignObject x={el.x} y={el.y} width={el.w} height={el.h} style={{ pointerEvents: "none" }}>
          <div className="txt" style={{ color: el.color, fontSize: el.fs }}>{el.text}</div>
        </foreignObject>
      );
    default: {
      const b = bbox(el);
      const common = { stroke: el.color, strokeWidth: el.w, fill: el.fill ?? "none", fillOpacity: el.fill ? 0.35 : undefined, strokeLinecap: "round" as const, strokeLinejoin: "round" as const };
      const label = (el.t === "rect" || el.t === "ellipse") && el.text && !editing ? (
        <foreignObject x={b.x} y={b.y} width={Math.max(b.w, 1)} height={Math.max(b.h, 1)} style={{ pointerEvents: "none" }}>
          <div className="shapetxt" style={{ color: el.color, fontSize: el.fs }}>{el.text}</div>
        </foreignObject>
      ) : null;
      return (
        <g>
          {el.t === "rect" && <rect x={b.x} y={b.y} width={b.w} height={b.h} {...common} />}
          {el.t === "ellipse" && <ellipse cx={b.x + b.w / 2} cy={b.y + b.h / 2} rx={b.w / 2} ry={b.h / 2} {...common} />}
          {(el.t === "line" || el.t === "arrow") && <line x1={el.x1} y1={el.y1} x2={el.x2} y2={el.y2} {...common} fill="none" />}
          {el.t === "arrow" && <path d={arrowHead(el)} {...common} fill="none" />}
          {label}
        </g>
      );
    }
  }
}

function Handles({ el, zoom }: { el: El; zoom: number }) {
  const b = bbox(el);
  const s = 1 / zoom;
  const resizable = el.t !== "pen" && el.t !== "hl";
  return (
    <g style={{ pointerEvents: "none" }}>
      <rect x={b.x - 2 * s} y={b.y - 2 * s} width={b.w + 4 * s} height={b.h + 4 * s} fill="none" stroke="#1a56db" strokeWidth={1.2 * s} strokeDasharray={`${4 * s} ${3 * s}`} />
      {resizable && <circle cx={b.x + b.w} cy={b.y + b.h} r={6 * s} fill="#fff" stroke="#1a56db" strokeWidth={1.5 * s} />}
    </g>
  );
}

function EditBox({ el, zoom, mx, ctx }: { el: El; zoom: number; mx: number; ctx: { current: Ctx } }) {
  const ref = useRef<HTMLTextAreaElement>(null);
  // focus after the click that created the box has finished, or the browser's own focus handling steals it back
  useEffect(() => { const t = setTimeout(() => { ref.current?.focus(); ref.current?.select(); }, 80); return () => clearTimeout(t); }, []);
  if (!hasText(el)) return null;
  const b = bbox(el);
  const isText = el.t === "text";
  const style: React.CSSProperties = isText
    ? { left: (el.x + mx) * zoom, top: el.y * zoom, width: el.w * zoom, minHeight: el.h * zoom }
    : { left: (b.x + mx) * zoom + 4, top: b.y * zoom + 4, width: Math.max(b.w * zoom - 8, 40), height: Math.max(b.h * zoom - 8, 24), textAlign: "center" };
  const done = () => {
    const t = ref.current;
    if (!t) return;
    ctx.current.finishEdit(el.id, t.value, isText ? t.scrollHeight / zoom : 0);
  };
  return (
    <textarea
      ref={ref}
      className="editbox"
      defaultValue={el.text}
      style={{ ...style, fontSize: el.fs * zoom, color: el.color }}
      onBlur={done}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Escape" || ((e.metaKey || e.ctrlKey) && e.key === "Enter")) (e.target as HTMLTextAreaElement).blur();
      }}
    />
  );
}

// ---------- the viewer ----------

type OutlineItem = { title: string; dest: unknown; items: OutlineItem[] };

export type Chapter = { title: string; page: number };

export function PdfAnnotator({ kind, refId, src, name, onReveal, chapters }: { kind: string; refId: string; src: string; name: string; onReveal: (reveal: boolean) => void; chapters?: Chapter[] }) {
  const [pdf, setPdf] = useState<PDFDocumentProxy | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [key, setKey] = useState("");
  const [doc, setDoc] = useState<Doc>(emptyDoc());
  const [sizes, setSizes] = useState<Size[]>([]);
  const [sizesDone, setSizesDone] = useState(false);
  const [zoom, setZoomState] = useState(1);
  const [page, setPage] = useState(1);
  const [tool, setTool] = useState<Tool>("view");
  const [color, setColor] = useState(COLORS[1]);
  const [widths, setWidths] = useState({ pen: 2.5, hl: 14, shape: 2 });
  const [fill, setFill] = useState(false);
  const [fs, setFs] = useState(14);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [pendingImage, setPendingImage] = useState<Ctx["pendingImage"]>(null);
  const [panel, setPanel] = useState<null | "contents" | "notes" | "marks" | "search">("contents");
  const [saveState, setSaveState] = useState<"saved" | "saving" | "error">("saved");
  const [msg, setMsg] = useState<string | null>(null);
  const [showExport, setShowExport] = useState(false);

  const scroller = useRef<HTMLDivElement>(null);
  const probeEl = useRef<HTMLSpanElement>(null);
  const past = useRef<El[][]>([]);
  const future = useRef<El[][]>([]);
  const newEdit = useRef<string | null>(null);
  const restored = useRef(false);
  const zoomSet = useRef(false);
  const fitRef = useRef(true); // true: the page width follows the window; false: the zoom the user chose
  const [fit, setFitState] = useState(true);
  const [margin, setMarginState] = useState(0);
  const marginRef = useRef(0);
  const pageW = (w: number) => w * (1 + 2 * marginRef.current); // width of a page plus its margins
  const setFit = (f: boolean) => { fitRef.current = f; setFitState(f); };
  const dirty = useRef(false);
  const loaded = useRef(false);
  const docRef = useRef(doc);
  docRef.current = doc;

  const els = doc.els;
  const setEls = useCallback((next: El[], pushHistory = true) => {
    setDoc((d) => {
      if (pushHistory) { past.current.push(d.els); if (past.current.length > 300) past.current.shift(); future.current = []; }
      return { ...d, els: next };
    });
    dirty.current = true;
  }, []);

  // ---- load the PDF and its saved marks ----
  useEffect(() => {
    let dead = false;
    let task: pdfjs.PDFDocumentLoadingTask | null = null;
    setErr(null); setPdf(null); setSizes([]); setSizesDone(false); restored.current = false; loaded.current = false;
    (async () => {
      try {
        const a = await annotLoad(kind, refId);
        if (dead) return;
        setKey(a.key);
        const d = normDoc(a.doc);
        setDoc(d);
        setPage(d.view.page);
        past.current = []; future.current = [];
        task = pdfjs.getDocument({ url: src, wasmUrl: "/pdfjs/wasm/", cMapUrl: "/pdfjs/cmaps/", cMapPacked: true, standardFontDataUrl: "/pdfjs/standard_fonts/", iccUrl: "/pdfjs/iccs/", disableAutoFetch: true, rangeChunkSize: 1 << 20 });
        const p = await task.promise;
        if (dead) { void task.destroy(); return; }
        setPdf(p);
        loaded.current = true;
      } catch (e) {
        if (!dead) setErr(String((e as Error).message ?? e));
      }
    })();
    return () => { dead = true; task?.destroy(); };
  }, [kind, refId, src]);

  // ---- page sizes (first page at once, the rest in the background) ----
  useEffect(() => {
    if (!pdf) return;
    let dead = false;
    (async () => {
      const out: Size[] = [];
      for (let i = 1; i <= pdf.numPages; i++) {
        const p = await pdf.getPage(i);
        const v = p.getViewport({ scale: 1 });
        out.push({ w: v.width, h: v.height });
        p.cleanup();
        if (dead) return;
        if (i === 1 || i % 60 === 0) setSizes(out.slice());
      }
      setSizes(out.slice());
      setSizesDone(true);
    })();
    return () => { dead = true; };
  }, [pdf]);

  const sizeOf = useCallback((i: number): Size => sizes[i] ?? sizes[0] ?? { w: 612, h: 792 }, [sizes]);

  // ---- zoom: saved zoom, else fit to width ----
  useEffect(() => {
    if (!sizes.length || zoomSet.current) return;
    const saved = docRef.current.view.zoom;
    const mgn = docRef.current.view.margin ?? (kind === "book" ? 0 : 0.15); // research papers get margins by default
    marginRef.current = mgn;
    setMarginState(mgn);
    const fitZ = ((scroller.current?.clientWidth ?? 900) - 32) / pageW(sizes[0].w);
    const f = docRef.current.view.fit !== false || saved == null;
    setFit(f);
    setZoomState(f ? Math.max(0.3, Math.min(4, fitZ)) : saved!);
    zoomSet.current = true;
  }, [sizes]);
  useEffect(() => { zoomSet.current = false; }, [kind, refId]);

  const setZoom = useCallback((z: number, keepFit = false) => {
    if (!keepFit) { fitRef.current = false; setFitState(false); }
    const el = scroller.current;
    const ratio = el ? el.scrollTop / Math.max(el.scrollHeight, 1) : 0;
    setZoomState(Math.max(0.3, Math.min(4, z)));
    dirty.current = true;
    requestAnimationFrame(() => requestAnimationFrame(() => { if (el) el.scrollTop = ratio * el.scrollHeight; }));
  }, []);
  const fitWidth = () => { setFit(true); setZoom(((scroller.current?.clientWidth ?? 900) - 32) / pageW(sizeOf(page - 1).w), true); };
  const changeMargin = (v: number) => { marginRef.current = v; setMarginState(v); if (fitRef.current) setZoom(((scroller.current?.clientWidth ?? 900) - 32) / pageW(sizeOf(page - 1).w), true); dirty.current = true; };
  // while "Fit width" is on, follow the window: full screen, a resized window, or a hidden side panel
  useEffect(() => {
    const el = scroller.current;
    if (!el || !sizes.length) return;
    const ro = new ResizeObserver(() => { if (fitRef.current) setZoom((el.clientWidth - 32) / pageW(sizes[0]?.w ?? 612), true); });
    ro.observe(el);
    return () => ro.disconnect();
  }, [sizes, setZoom]);
  // pinch on the trackpad or Ctrl/Cmd + wheel zooms the page
  const zoomRef = useRef(zoom);
  zoomRef.current = zoom;
  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      setZoom(zoomRef.current * Math.exp(-e.deltaY * 0.01));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [setZoom, pdf]);

  // ---- scroll to a page; restore the last page once sizes are known ----
  const goPage = useCallback((n: number) => {
    const el = scroller.current?.children[0]?.children[Math.max(0, n - 1)] as HTMLElement | undefined;
    if (el && scroller.current) scroller.current.scrollTo({ top: el.offsetTop - 10 });
  }, []);
  useEffect(() => {
    if (sizesDone && !restored.current && zoomSet.current) {
      restored.current = true;
      requestAnimationFrame(() => requestAnimationFrame(() => goPage(docRef.current.view.page)));
    }
  }, [sizesDone, zoom, goPage]);

  const onScroll = useCallback(() => {
    const s = scroller.current;
    const list = s?.children[0]?.children;
    if (!s || !list) return;
    const y = s.scrollTop + s.clientHeight * 0.35;
    let lo = 0, hi = list.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if ((list[mid] as HTMLElement).offsetTop <= y) lo = mid; else hi = mid - 1;
    }
    setPage(lo + 1);
  }, []);
  useEffect(() => {
    if (!restored.current) return;
    setDoc((d) => (d.view.page === page && d.view.zoom === zoom && d.view.fit === fit && d.view.margin === margin ? d : { ...d, view: { page, zoom, fit, margin } }));
    dirty.current = true;
  }, [page, zoom, fit, margin]);

  // ---- autosave ----
  useEffect(() => {
    if (!key || !loaded.current || !dirty.current) return;
    setSaveState("saving");
    const t = setTimeout(() => {
      annotSave(key, docRef.current).then(() => { dirty.current = false; setSaveState("saved"); }).catch(() => setSaveState("error"));
    }, 700);
    return () => clearTimeout(t);
  }, [doc, key]);
  useEffect(() => () => {
    if (dirty.current && key) annotSave(key, docRef.current).catch(() => {});
  }, [key]);

  // ---- history ----
  const undo = useCallback(() => {
    const prev = past.current.pop();
    if (!prev) return;
    future.current.push(docRef.current.els);
    setDoc((d) => ({ ...d, els: prev }));
    dirty.current = true;
    setSelectedId(null);
  }, []);
  const redo = useCallback(() => {
    const nxt = future.current.pop();
    if (!nxt) return;
    past.current.push(docRef.current.els);
    setDoc((d) => ({ ...d, els: nxt }));
    dirty.current = true;
  }, []);

  // ---- context handed to pages ----
  const width = tool === "pen" ? widths.pen : tool === "hl" ? widths.hl : widths.shape;
  const ctx = useRef<Ctx>(null as unknown as Ctx);
  ctx.current = {
    tool, color, width, fill, fs, zoom, selectedId, pendingImage, key,
    add: (el, edit) => {
      setEls([...docRef.current.els, el]);
      if (edit) { newEdit.current = el.id; setEditingId(el.id); }
      else if (el.t !== "pen" && el.t !== "hl") setSelectedId(null);
    },
    replace: (el) => setEls(docRef.current.els.map((e) => (e.id === el.id ? el : e))),
    remove: (ids) => { setEls(docRef.current.els.filter((e) => !ids.includes(e.id))); setSelectedId((s) => (s && ids.includes(s) ? null : s)); },
    select: (id) => setSelectedId(id),
    edit: (id) => { newEdit.current = null; setEditingId(id); },
    finishEdit: (id, text, h) => {
      setEditingId(null);
      const cur = docRef.current.els.find((e) => e.id === id);
      if (!cur) return;
      if (cur.t === "text" && !text.trim()) {
        // an empty new text box leaves no trace; an emptied old one is deleted (undoable)
        if (newEdit.current === id) { const prev = past.current.pop(); if (prev) setDoc((d) => ({ ...d, els: prev })); }
        else setEls(docRef.current.els.filter((e) => e.id !== id));
      } else if (cur.t === "text") {
        const next: TextBox = { ...cur, text, h: Math.max(cur.fs * 1.4, h) };
        if (newEdit.current === id) setDoc((d) => ({ ...d, els: d.els.map((e) => (e.id === id ? next : e)) }));
        else setEls(docRef.current.els.map((e) => (e.id === id ? next : e)));
        dirty.current = true;
      } else if (cur.t === "rect" || cur.t === "ellipse") {
        if (cur.text !== text) setEls(docRef.current.els.map((e) => (e.id === id ? { ...cur, text } : e)));
      }
      newEdit.current = null;
    },
    probe: (type, pressure) => { if (probeEl.current) probeEl.current.textContent = `${type} · pressure ${pressure.toFixed(2)}`; },
    doneImage: () => { setPendingImage(null); setTool("select"); },
  };

  // ---- recolour / delete the selected mark ----
  const selected = els.find((e) => e.id === selectedId) ?? null;
  const pickColor = (c: string) => {
    setColor(c);
    if (selected && selected.t !== "image") setEls(els.map((e) => (e.id === selected.id ? { ...e, color: c } : e)));
  };
  const deleteSelected = useCallback(() => {
    if (selectedId) { setEls(docRef.current.els.filter((e) => e.id !== selectedId)); setSelectedId(null); }
  }, [selectedId, setEls]);

  // ---- images: file chooser or paste ----
  const addImageBlob = useCallback(async (blob: Blob) => {
    try {
      const bmp = await createImageBitmap(blob);
      const scale = Math.min(1, 2000 / Math.max(bmp.width, bmp.height));
      const c = document.createElement("canvas");
      c.width = Math.round(bmp.width * scale); c.height = Math.round(bmp.height * scale);
      c.getContext("2d")!.drawImage(bmp, 0, 0, c.width, c.height);
      const png = blob.type !== "image/jpeg";
      const out: Blob = await new Promise((res, rej) => c.toBlob((b) => (b ? res(b) : rej(new Error("could not read that image"))), png ? "image/png" : "image/jpeg", 0.92));
      const file = await annotUploadImage(key, out, png ? "png" : "jpg");
      setPendingImage({ file, w: c.width, h: c.height });
      setTool("image");
      setMsg("Click on the page where the picture should go.");
    } catch (e) { setMsg(String((e as Error).message ?? e)); }
  }, [key]);
  useEffect(() => {
    const f = (e: ClipboardEvent) => {
      if ((e.target as HTMLElement)?.tagName === "TEXTAREA" || (e.target as HTMLElement)?.tagName === "INPUT") return;
      const it = [...(e.clipboardData?.items ?? [])].find((i) => i.type.startsWith("image/"));
      const file = it?.getAsFile();
      if (file) { e.preventDefault(); void addImageBlob(file); }
    };
    addEventListener("paste", f);
    return () => removeEventListener("paste", f);
  }, [addImageBlob]);

  // ---- keyboard: every tool has a key so the tablet's buttons can be set to send them ----
  useEffect(() => {
    const f = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
      const k = e.key.toLowerCase();
      if ((e.metaKey || e.ctrlKey) && k === "z") { e.preventDefault(); if (e.shiftKey) redo(); else undo(); return; }
      if ((e.metaKey || e.ctrlKey) && k === "y") { e.preventDefault(); redo(); return; }
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const byKey: Record<string, Tool> = { g: "view", v: "select", p: "pen", h: "hl", e: "eraser", r: "rect", o: "ellipse", l: "line", a: "arrow", t: "text" };
      if (byKey[k]) { e.preventDefault(); setTool(byKey[k]); return; }
      if (k === "delete" || k === "backspace") { if (selectedId) { e.preventDefault(); deleteSelected(); } return; }
      if (k === "escape") { setSelectedId(null); setPendingImage(null); setTool("select"); return; }
      if (k === "=" || k === "+") { e.preventDefault(); setZoom(zoom * 1.15); return; }
      if (k === "-") { e.preventDefault(); setZoom(zoom / 1.15); return; }
      if (k === "[" || k === "]") {
        const d = k === "]" ? 1.25 : 0.8;
        setWidths((w) => (tool === "pen" ? { ...w, pen: Math.max(0.5, Math.min(24, w.pen * d)) } : tool === "hl" ? { ...w, hl: Math.max(4, Math.min(48, w.hl * d)) } : { ...w, shape: Math.max(0.5, Math.min(16, w.shape * d)) }));
        return;
      }
      if (k === "pagedown" && !e.shiftKey) { e.preventDefault(); goPage(Math.min((pdf?.numPages ?? 1), page + 1)); }
      if (k === "pageup") { e.preventDefault(); goPage(Math.max(1, page - 1)); }
    };
    addEventListener("keydown", f);
    return () => removeEventListener("keydown", f);
  }, [undo, redo, selectedId, deleteSelected, zoom, setZoom, tool, goPage, page, pdf]);

  // ---- per-page element lists with stable identity, so only the page you draw on re-renders ----
  const prevBy = useRef<Map<number, El[]>>(new Map());
  const byPage = useMemo(() => {
    const m = new Map<number, El[]>();
    for (const e of els) { const l = m.get(e.page); if (l) l.push(e); else m.set(e.page, [e]); }
    for (const [p, l] of m) {
      const old = prevBy.current.get(p);
      if (old && old.length === l.length && old.every((x, i) => x === l[i])) m.set(p, old);
    }
    prevBy.current = m;
    return m;
  }, [els]);
  const none = useRef<El[]>([]).current;

  const bookmarked = doc.bookmarks.includes(page);
  const toggleBookmark = () => { setDoc((d) => ({ ...d, bookmarks: d.bookmarks.includes(page) ? d.bookmarks.filter((b) => b !== page) : [...d.bookmarks, page].sort((a, b) => a - b) })); dirty.current = true; };

  const pickFill = tool === "rect" || tool === "ellipse";
  const palette = tool === "hl" ? HL_COLORS : COLORS;

  return (
    <div className="annot">
      <div className="atools">
        <div className="agrp">
          {TOOLS.map((x) => (
            <button key={x.t} className={"btn sm" + (tool === x.t ? " primary" : "")} title={x.tip} onClick={() => { setTool(x.t); }}>{x.label}</button>
          ))}
          <label className="btn sm" title="Add a picture from a file. You can also paste one with Cmd-V.">
            Image…
            <input type="file" accept="image/*" hidden onChange={(e) => { const f = e.target.files?.[0]; if (f) void addImageBlob(f); e.target.value = ""; }} />
          </label>
        </div>
        <div className="agrp">
          {palette.map((c) => <button key={c} className={"swatch" + (color === c ? " on" : "")} style={{ background: c }} aria-label={`Colour ${c}`} onClick={() => pickColor(c)} />)}
          <input type="color" value={color} onChange={(e) => pickColor(e.target.value)} aria-label="Pick a colour" title="Any colour" className="colorin" />
          {(tool === "pen" || tool === "hl" || tool === "rect" || tool === "ellipse" || tool === "line" || tool === "arrow") && (
            <label className="small" title="Thickness. [ and ] change it from the keyboard.">
              Thickness <input type="range" min={tool === "hl" ? 4 : 0.5} max={tool === "hl" ? 48 : 16} step={0.5} value={width} onChange={(e) => { const v = Number(e.target.value); setWidths((w) => (tool === "pen" ? { ...w, pen: v } : tool === "hl" ? { ...w, hl: v } : { ...w, shape: v })); }} />
            </label>
          )}
          {pickFill && <label className="small"><input type="checkbox" checked={fill} onChange={(e) => setFill(e.target.checked)} /> Fill</label>}
          {(tool === "text" || selected?.t === "text" || selected?.t === "rect" || selected?.t === "ellipse" || pickFill) && (
            <label className="small">Text size <input type="number" min={6} max={72} value={fs} style={{ width: "3.6em" }} onChange={(e) => { const v = Math.max(6, Math.min(72, Number(e.target.value) || 14)); setFs(v); if (selected && hasText(selected)) setEls(els.map((x) => (x.id === selected.id ? { ...selected, fs: v } : x))); }} /></label>
          )}
        </div>
        <div className="agrp">
          <button className="btn sm" onClick={undo} disabled={!past.current.length} title="Undo (Cmd-Z)">Undo</button>
          <button className="btn sm" onClick={redo} disabled={!future.current.length} title="Redo (Cmd-Shift-Z)">Redo</button>
          <button className="btn sm" onClick={deleteSelected} disabled={!selectedId} title="Delete the selected mark (Delete)">Delete</button>
        </div>
        <div className="agrp">
          <button className="btn sm" onClick={() => setZoom(zoom / 1.15)} title="Zoom out (-)">−</button>
          <span className="small">{Math.round(zoom * 100)}%</span>
          <button className="btn sm" onClick={() => setZoom(zoom * 1.15)} title="Zoom in (+)">+</button>
          <button className={"btn sm" + (fit ? " primary" : "")} onClick={fitWidth} title="Keep the page as wide as the window">Fit width</button>
          <label className="small" title="Blank space on each side of the page, to write notes in">Margins <select value={margin} onChange={(e) => changeMargin(Number(e.target.value))} aria-label="Page margins">
            <option value={0}>None</option><option value={0.1}>10%</option><option value={0.15}>15%</option><option value={0.2}>20%</option>
          </select></label>
        </div>
        <div className="agrp">
          {chapters?.length ? <span className="small muted" title="The paper you are in">{chapters.reduce((a, c) => (c.page <= page ? c : a), chapters[0]).title.slice(0, 48)}</span> : null}
          <input className="pagein" type="number" min={1} max={pdf?.numPages ?? 1} value={page} onChange={(e) => goPage(Number(e.target.value) || 1)} aria-label="Page" />
          <span className="small">/ {pdf?.numPages ?? "–"}</span>
          <button className={"btn sm" + (bookmarked ? " primary" : "")} onClick={toggleBookmark} title="Bookmark this page">★</button>
        </div>
        <div className="agrp">
          <button className="btn sm" onClick={() => setShowExport(!showExport)} title="Save a copy of the PDF with your marks drawn on it">Export…</button>
          <button className="btn sm" onClick={() => onReveal(false)}>Open in Preview</button>
          <button className="btn sm" onClick={() => onReveal(true)}>Show in Finder</button>
        </div>
        <div className="agrp small muted">
          <span title="What the pen is reporting. Check this to see whether your tablet sends pressure.">Pen: <span ref={probeEl}>–</span></span>
          <span className={saveState === "error" ? "bad" : ""}>{saveState === "saved" ? "Saved" : saveState === "saving" ? "Saving…" : "Could not save"}</span>
        </div>
      </div>
      {msg && <div className="muted small ok" style={{ margin: ".2rem 0" }}>{msg} <button className="btn sm" onClick={() => setMsg(null)}>OK</button></div>}
      {showExport && pdf && <ExportPanel name={name} total={pdf.numPages} src={src} els={els} margin={margin} keyId={key} onClose={() => setShowExport(false)} setMsg={setMsg} />}

      <div className="abody">
        <aside className="aside">
          <div className="atabs">
            {(["contents", "notes", "marks", "search"] as const).map((p) => <button key={p} className={"btn sm" + (panel === p ? " primary" : "")} onClick={() => setPanel(panel === p ? null : p)}>{p === "marks" ? "Bookmarks" : p[0].toUpperCase() + p.slice(1)}</button>)}
          </div>
          {panel === "contents" && pdf && (chapters?.length ? <Chapters chapters={chapters} page={page} goPage={goPage} /> : <Contents pdf={pdf} goPage={goPage} />)}
          {panel === "notes" && <Notes els={els} goPage={goPage} select={(id, p) => { setSelectedId(id); setTool("select"); goPage(p); }} />}
          {panel === "marks" && (
            <div className="alist">
              {doc.bookmarks.length === 0 && <div className="muted small">No bookmarks yet. Press ★ on a page.</div>}
              {doc.bookmarks.map((b) => <button key={b} className="dirrow" onClick={() => goPage(b)}><span>★ Page {b}</span></button>)}
            </div>
          )}
          {panel === "search" && pdf && <Search pdf={pdf} goPage={goPage} />}
        </aside>
        <div className="ascroll" ref={scroller} onScroll={onScroll}>
          {err && <div className="err">{err}</div>}
          {!pdf && !err && <p className="muted" style={{ padding: "1rem" }}>Opening…</p>}
          {pdf && (
            <div className="pages">
              {Array.from({ length: pdf.numPages }, (_, i) => (
                <PageView key={i} pdf={pdf} n={i + 1} size={sizeOf(i)} zoom={zoom} mg={margin} els={byPage.get(i + 1) ?? none} ctx={ctx} root={scroller} tool={tool} selectedId={selectedId} editingId={editingId} />
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// ---------- sidebar pieces ----------

function Chapters({ chapters, page, goPage }: { chapters: Chapter[]; page: number; goPage: (n: number) => void }) {
  const cur = chapters.reduce((a, c, i) => (c.page <= page ? i : a), 0);
  return (
    <div className="alist">
      {chapters.map((c, i) => (
        <button key={c.page} className={"dirrow" + (i === cur ? " on" : "")} onClick={() => goPage(c.page)} title={c.title} style={i === cur ? { borderLeft: "4px solid var(--accent)" } : undefined}>
          <span style={{ whiteSpace: "normal" }}><b>{i + 1}.</b> {c.title}</span><span className="muted small">p. {c.page}</span>
        </button>
      ))}
    </div>
  );
}

function Contents({ pdf, goPage }: { pdf: PDFDocumentProxy; goPage: (n: number) => void }) {
  const [items, setItems] = useState<OutlineItem[] | null>(null);
  useEffect(() => { let dead = false; pdf.getOutline().then((o) => !dead && setItems((o as OutlineItem[] | null) ?? [])).catch(() => !dead && setItems([])); return () => { dead = true; }; }, [pdf]);
  const open = async (dest: unknown) => {
    try {
      const d = typeof dest === "string" ? await pdf.getDestination(dest) : (dest as unknown[]);
      if (!d) return;
      const ref = d[0];
      const idx = typeof ref === "number" ? ref : await pdf.getPageIndex(ref as never);
      goPage(idx + 1);
    } catch { /* a broken link in the book's own outline */ }
  };
  if (!items) return <div className="muted small">Reading the contents…</div>;
  if (!items.length) return <div className="muted small">This PDF has no table of contents.</div>;
  return <div className="alist">{items.map((it, i) => <OutlineNode key={i} it={it} depth={0} open={open} />)}</div>;
}

function OutlineNode({ it, depth, open }: { it: OutlineItem; depth: number; open: (d: unknown) => void }) {
  const [x, setX] = useState(false);
  return (
    <div>
      <div className="outrow" style={{ paddingLeft: depth * 12 }}>
        {it.items.length > 0 ? <button className="tw" onClick={() => setX(!x)} aria-label={x ? "Collapse" : "Expand"}>{x ? "▾" : "▸"}</button> : <span className="tw" />}
        <button className="outlink" onClick={() => open(it.dest)} title={it.title}>{it.title}</button>
      </div>
      {x && it.items.map((c, i) => <OutlineNode key={i} it={c} depth={depth + 1} open={open} />)}
    </div>
  );
}

const LABEL: Record<string, string> = { pen: "Pen", hl: "Highlight", rect: "Box", ellipse: "Oval", line: "Line", arrow: "Arrow", text: "Text", image: "Picture" };

function Notes({ els, goPage, select }: { els: El[]; goPage: (n: number) => void; select: (id: string, page: number) => void }) {
  const pages = [...new Set(els.map((e) => e.page))].sort((a, b) => a - b);
  if (!pages.length) return <div className="muted small">No marks yet.</div>;
  return (
    <div className="alist">
      {pages.map((p) => (
        <div key={p}>
          <button className="dirrow" onClick={() => goPage(p)}><b>Page {p}</b><span className="muted small">{els.filter((e) => e.page === p).length} marks</span></button>
          {els.filter((e) => e.page === p).map((e) => {
            const text = e.t === "text" || e.t === "rect" || e.t === "ellipse" ? e.text : "";
            return <button key={e.id} className="dirrow sub" onClick={() => select(e.id, p)}><span><span className="dot" style={{ background: e.color }} /> {LABEL[e.t]}{text ? `: ${text.slice(0, 40)}` : ""}</span></button>;
          })}
        </div>
      ))}
    </div>
  );
}

function Search({ pdf, goPage }: { pdf: PDFDocumentProxy; goPage: (n: number) => void }) {
  const [q, setQ] = useState("");
  const [hits, setHits] = useState<{ page: number; snip: string }[]>([]);
  const [busy, setBusy] = useState<number | null>(null);
  const token = useRef(0);
  const run = async () => {
    const my = ++token.current;
    const needle = q.trim().toLowerCase();
    setHits([]);
    if (!needle) { setBusy(null); return; }
    const out: { page: number; snip: string }[] = [];
    for (let i = 1; i <= pdf.numPages; i++) {
      if (token.current !== my) return;
      setBusy(i);
      const page = await pdf.getPage(i);
      const tc = await page.getTextContent();
      const text = tc.items.map((it) => ("str" in it ? it.str : "")).join(" ").replace(/\s+/g, " ");
      const low = text.toLowerCase();
      let at = low.indexOf(needle);
      let per = 0;
      while (at >= 0 && per < 3) {
        out.push({ page: i, snip: text.slice(Math.max(0, at - 30), at + needle.length + 40) });
        per++;
        at = low.indexOf(needle, at + needle.length);
      }
      page.cleanup();
      if (i % 10 === 0 || out.length >= 200) setHits(out.slice());
      if (out.length >= 200) break;
    }
    setHits(out.slice());
    setBusy(null);
  };
  return (
    <div className="alist">
      <div className="controls">
        <input className="search" value={q} placeholder="Find in this PDF" onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.key === "Enter" && void run()} />
        <button className="btn sm" onClick={() => void run()}>Find</button>
      </div>
      {busy != null && <div className="muted small">Searching page {busy}… <button className="btn sm" onClick={() => { token.current++; setBusy(null); }}>Stop</button></div>}
      {busy == null && q && hits.length === 0 && <div className="muted small">No matches, or the PDF is a scan with no text layer.</div>}
      {hits.map((h, i) => <button key={i} className="dirrow sub" onClick={() => goPage(h.page)}><span className="small">p.{h.page} · …{h.snip}…</span></button>)}
    </div>
  );
}

function ExportPanel({ name, total, src, els, margin, keyId, onClose, setMsg }: { name: string; total: number; src: string; els: El[]; margin: number; keyId: string; onClose: () => void; setMsg: (m: string | null) => void }) {
  const [from, setFrom] = useState(1);
  const [to, setTo] = useState(total);
  const [busy, setBusy] = useState(false);
  const [out, setOut] = useState<{ file: string; path: string; skipped: string[] } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const go = async () => {
    setBusy(true); setErr(null); setOut(null);
    try {
      const bytes = await (await fetch(src)).arrayBuffer();
      const r = await exportAnnotated(bytes, els, async (file) => {
        const b = await (await fetch(annotImageUrl(keyId, file))).arrayBuffer();
        return { bytes: b, png: file.toLowerCase().endsWith(".png") };
      }, from, to, margin);
      const saved = await annotExport(`${name}_p${from}-${to}`, r.bytes);
      setOut({ ...saved, skipped: r.skipped });
      setMsg(null);
    } catch (e) { setErr(String((e as Error).message ?? e)); } finally { setBusy(false); }
  };
  return (
    <div className="panel">
      <div className="controls">
        <b>Export a copy with your marks</b>
        <span className="small">pages</span>
        <input type="number" min={1} max={total} value={from} style={{ width: "5em" }} onChange={(e) => setFrom(Math.max(1, Number(e.target.value) || 1))} />
        <span className="small">to</span>
        <input type="number" min={1} max={total} value={to} style={{ width: "5em" }} onChange={(e) => setTo(Math.min(total, Number(e.target.value) || total))} />
        <button className="btn primary" onClick={() => void go()} disabled={busy}>{busy ? "Exporting…" : "Export"}</button>
        <span className="spacer" />
        <button className="btn sm" onClick={onClose}>Close</button>
      </div>
      <div className="muted small">Writes a new PDF into CourseShelf/annotations/exports/. The original is not touched. Typed text is exported in Helvetica, so characters outside Latin-1 (Greek letters, for example) become “?”. For a whole book, choose a page range.</div>
      {err && <div className="err">{err}</div>}
      {out && (
        <div className="small ok">
          Saved <code>{out.path}</code> <button className="btn sm" onClick={() => void annotReveal(out.file)}>Show in Finder</button>
          {out.skipped.map((s) => <div key={s} style={{ color: "var(--warn)" }}>{s}</div>)}
        </div>
      )}
    </div>
  );
}
