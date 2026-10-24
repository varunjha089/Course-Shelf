// Annotation model: the shapes saved next to a PDF, hit testing, stroke outlines and export into a copy of the PDF.
// Coordinates are page units: the page at 100% zoom, origin top-left, y down (what pdf.js calls a scale-1 viewport).

import getStroke from "perfect-freehand";
import { BlendMode, LineCapStyle, PDFDocument, PDFFont, PDFImage, StandardFonts, rgb } from "pdf-lib";

export type Pt = [number, number, number]; // x, y, pressure

interface Base {
  id: string;
  page: number; // 1-based
  color: string; // #rrggbb
}
export interface Stroke extends Base { t: "pen" | "hl"; w: number; pts: Pt[]; pr: boolean }
export interface Shape extends Base { t: "rect" | "ellipse" | "line" | "arrow"; x1: number; y1: number; x2: number; y2: number; w: number; fill: string | null; text: string; fs: number }
export interface TextBox extends Base { t: "text"; x: number; y: number; w: number; h: number; text: string; fs: number }
export interface ImageEl extends Base { t: "image"; x: number; y: number; w: number; h: number; file: string }
export type El = Stroke | Shape | TextBox | ImageEl;

export interface Doc {
  v: 1;
  els: El[];
  bookmarks: number[];
  view: { page: number; zoom: number | null; fit?: boolean; margin?: number }; // margin: blank space on each side of every page, as a fraction of the page width
}

export const emptyDoc = (): Doc => ({ v: 1, els: [], bookmarks: [], view: { page: 1, zoom: null } });

export const uid = () => Math.random().toString(36).slice(2, 10);

export function normDoc(d: unknown): Doc {
  const o = (d ?? {}) as Partial<Doc>;
  return {
    v: 1,
    els: Array.isArray(o.els) ? (o.els as El[]) : [],
    bookmarks: Array.isArray(o.bookmarks) ? o.bookmarks.filter((n) => typeof n === "number") : [],
    view: {
      page: o.view?.page ?? 1,
      zoom: typeof o.view?.zoom === "number" ? o.view.zoom : null,
      ...(typeof o.view?.fit === "boolean" ? { fit: o.view.fit } : {}),
      ...(typeof o.view?.margin === "number" ? { margin: Math.max(0, Math.min(0.3, o.view.margin)) } : {}),
    },
  };
}

// ---------- geometry ----------

export function segDist(px: number, py: number, x1: number, y1: number, x2: number, y2: number): number {
  const dx = x2 - x1, dy = y2 - y1;
  const l2 = dx * dx + dy * dy;
  let t = l2 === 0 ? 0 : ((px - x1) * dx + (py - y1) * dy) / l2;
  t = Math.max(0, Math.min(1, t));
  return Math.hypot(px - (x1 + t * dx), py - (y1 + t * dy));
}

const lo = (a: number, b: number) => Math.min(a, b);
const hi = (a: number, b: number) => Math.max(a, b);

export function bbox(el: El): { x: number; y: number; w: number; h: number } {
  switch (el.t) {
    case "pen":
    case "hl": {
      let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
      for (const p of el.pts) { x0 = lo(x0, p[0]); y0 = lo(y0, p[1]); x1 = hi(x1, p[0]); y1 = hi(y1, p[1]); }
      const m = el.w / 2;
      return { x: x0 - m, y: y0 - m, w: x1 - x0 + el.w, h: y1 - y0 + el.w };
    }
    case "text":
    case "image":
      return { x: el.x, y: el.y, w: el.w, h: el.h };
    default:
      return { x: lo(el.x1, el.x2), y: lo(el.y1, el.y2), w: Math.abs(el.x2 - el.x1), h: Math.abs(el.y2 - el.y1) };
  }
}

export function hit(el: El, x: number, y: number, r: number): boolean {
  switch (el.t) {
    case "pen":
    case "hl": {
      const rr = r + el.w / 2;
      if (el.pts.length === 1) return Math.hypot(x - el.pts[0][0], y - el.pts[0][1]) <= rr;
      for (let i = 1; i < el.pts.length; i++) {
        if (segDist(x, y, el.pts[i - 1][0], el.pts[i - 1][1], el.pts[i][0], el.pts[i][1]) <= rr) return true;
      }
      return false;
    }
    case "line":
    case "arrow":
      return segDist(x, y, el.x1, el.y1, el.x2, el.y2) <= r + el.w / 2;
    case "rect": {
      const b = bbox(el);
      const inside = x >= b.x - r && x <= b.x + b.w + r && y >= b.y - r && y <= b.y + b.h + r;
      if (el.fill || el.text) return inside;
      const rr = r + el.w / 2;
      return (
        segDist(x, y, b.x, b.y, b.x + b.w, b.y) <= rr || segDist(x, y, b.x, b.y + b.h, b.x + b.w, b.y + b.h) <= rr ||
        segDist(x, y, b.x, b.y, b.x, b.y + b.h) <= rr || segDist(x, y, b.x + b.w, b.y, b.x + b.w, b.y + b.h) <= rr
      );
    }
    case "ellipse": {
      const b = bbox(el);
      const rx = Math.max(b.w / 2, 1), ry = Math.max(b.h / 2, 1);
      const dx = (x - (b.x + rx)) / rx, dy = (y - (b.y + ry)) / ry;
      const d = Math.sqrt(dx * dx + dy * dy);
      if (el.fill || el.text) return d <= 1 + r / Math.min(rx, ry);
      return Math.abs(d - 1) * Math.min(rx, ry) <= r + el.w / 2;
    }
    default:
      return x >= el.x - r && x <= el.x + el.w + r && y >= el.y - r && y <= el.y + el.h + r;
  }
}

export function moveEl<T extends El>(el: T, dx: number, dy: number): T {
  switch (el.t) {
    case "pen":
    case "hl":
      return { ...el, pts: el.pts.map((p) => [p[0] + dx, p[1] + dy, p[2]] as Pt) };
    case "text":
    case "image":
      return { ...el, x: el.x + dx, y: el.y + dy };
    default:
      return { ...el, x1: el.x1 + dx, y1: el.y1 + dy, x2: el.x2 + dx, y2: el.y2 + dy };
  }
}

/** Drag the bottom-right handle to (x, y). Strokes cannot be resized. */
export function resizeEl(el: El, x: number, y: number): El {
  switch (el.t) {
    case "pen":
    case "hl":
      return el;
    case "text":
      return { ...el, w: Math.max(30, x - el.x), h: Math.max(el.fs * 1.3, y - el.y) };
    case "image": {
      const ratio = el.h / el.w;
      const w = Math.max(20, x - el.x);
      return { ...el, w, h: w * ratio };
    }
    default:
      return { ...el, x2: x, y2: y };
  }
}

export const isShape = (e: El): e is Shape => e.t === "rect" || e.t === "ellipse" || e.t === "line" || e.t === "arrow";

export function hasText(el: El): el is TextBox | Shape {
  return el.t === "text" || el.t === "rect" || el.t === "ellipse";
}

// ---------- drawing helpers shared by the screen and the export ----------

/** Outline polygon of a pressure-sensitive pen stroke as an SVG path. */
export function penPath(pts: Pt[], size: number, pressure: boolean): string {
  const o = getStroke(pts.map((p) => [p[0], p[1], p[2]]), {
    size,
    thinning: pressure ? 0.65 : 0.35,
    smoothing: 0.55,
    streamline: 0.45,
    simulatePressure: !pressure,
    last: true,
  });
  if (!o.length) return "";
  let d = `M${o[0][0].toFixed(2)},${o[0][1].toFixed(2)}`;
  for (let i = 1; i < o.length; i++) {
    const [x0, y0] = o[i - 1], [x1, y1] = o[i];
    d += `Q${x0.toFixed(2)},${y0.toFixed(2)} ${((x0 + x1) / 2).toFixed(2)},${((y0 + y1) / 2).toFixed(2)}`;
  }
  return d + "Z";
}

export function polyline(pts: Pt[]): string {
  if (pts.length === 1) return `M${pts[0][0]},${pts[0][1]}L${pts[0][0] + 0.01},${pts[0][1]}`;
  return pts.map((p, i) => `${i ? "L" : "M"}${p[0].toFixed(2)},${p[1].toFixed(2)}`).join("");
}

export function arrowHead(el: Shape): string {
  const a = Math.atan2(el.y2 - el.y1, el.x2 - el.x1);
  const L = 8 + el.w * 2;
  const p = (da: number) => `${(el.x2 - L * Math.cos(a + da)).toFixed(2)},${(el.y2 - L * Math.sin(a + da)).toFixed(2)}`;
  return `M${p(0.45)}L${el.x2},${el.y2}L${p(-0.45)}`;
}

// ---------- export: a copy of the PDF with the marks drawn on ----------

function hexRgb(h: string) {
  const n = parseInt(h.replace("#", ""), 16);
  return rgb(((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255);
}

/** Helvetica only covers Latin-1; anything else is written as "?". */
function latin(s: string): string {
  return Array.from(s).map((c) => (c === "\n" || c.codePointAt(0)! < 256 ? c : "?")).join("");
}

function wrap(text: string, font: PDFFont, size: number, maxW: number): string[] {
  const out: string[] = [];
  for (const para of latin(text).split("\n")) {
    let line = "";
    for (const word of para.split(" ")) {
      const t = line ? line + " " + word : word;
      if (line && font.widthOfTextAtSize(t, size) > maxW) { out.push(line); line = word; } else line = t;
    }
    out.push(line);
  }
  return out;
}

export interface ExportResult { bytes: Uint8Array; skipped: string[] }

export async function exportAnnotated(
  srcBytes: ArrayBuffer,
  els: El[],
  imageBytes: (file: string) => Promise<{ bytes: ArrayBuffer; png: boolean }>,
  from: number,
  to: number,
  margin = 0,
): Promise<ExportResult> {
  const src = await PDFDocument.load(srcBytes, { ignoreEncryption: true });
  const total = src.getPageCount();
  from = Math.max(1, from); to = Math.min(total, to);
  let out = src;
  let idx = (p: number) => p - 1;
  if (from !== 1 || to !== total) {
    out = await PDFDocument.create();
    const pages = await out.copyPages(src, Array.from({ length: to - from + 1 }, (_, i) => from - 1 + i));
    pages.forEach((p) => out.addPage(p));
    idx = (p) => p - from;
  }
  if (margin > 0) {
    // widen every exported page by the same blank margins the viewer shows; the page content does not move
    for (let p = from; p <= to; p++) {
      const pg = out.getPage(idx(p));
      if (pg.getRotation().angle % 360 !== 0) continue;
      const cb = pg.getCropBox();
      const m = cb.width * margin;
      pg.setCropBox(cb.x - m, cb.y, cb.width + 2 * m, cb.height);
      pg.setMediaBox(cb.x - m, cb.y, cb.width + 2 * m, cb.height);
    }
  }
  const font = await out.embedFont(StandardFonts.Helvetica);
  const imgs = new Map<string, PDFImage>();
  const skipped: string[] = [];

  for (const el of els) {
    if (el.page < from || el.page > to) continue;
    const page = out.getPage(idx(el.page));
    if (page.getRotation().angle % 360 !== 0) {
      const m = `page ${el.page} is rotated; its marks were left out`;
      if (!skipped.includes(m)) skipped.push(m);
      continue;
    }
    const cb = page.getCropBox();
    const ox = cb.x, oy = cb.y + cb.height; // PDF coords of the page's top-left
    const X = (x: number) => ox + x, Y = (y: number) => oy - y;
    const col = hexRgb(el.color);

    if (el.t === "pen") {
      page.drawSvgPath(penPath(el.pts, el.w, el.pr), { x: ox, y: oy, color: col, borderWidth: 0 });
    } else if (el.t === "hl") {
      page.drawSvgPath(polyline(el.pts), { x: ox, y: oy, borderColor: col, borderWidth: el.w, borderOpacity: 0.35, borderLineCap: LineCapStyle.Round, blendMode: BlendMode.Multiply });
    } else if (el.t === "image") {
      let im = imgs.get(el.file);
      if (!im) {
        const { bytes, png } = await imageBytes(el.file);
        im = png ? await out.embedPng(bytes) : await out.embedJpg(bytes);
        imgs.set(el.file, im);
      }
      page.drawImage(im, { x: X(el.x), y: Y(el.y + el.h), width: el.w, height: el.h });
    } else if (el.t === "text") {
      const lines = wrap(el.text, font, el.fs, el.w);
      lines.forEach((ln, i) => page.drawText(ln, { x: X(el.x), y: Y(el.y + el.fs * (1.05 + 1.2 * i)), size: el.fs, font, color: col }));
    } else if (isShape(el)) {
      const b = bbox(el);
      if (el.t === "rect") {
        page.drawRectangle({ x: X(b.x), y: Y(b.y + b.h), width: b.w, height: b.h, borderColor: col, borderWidth: el.w, color: el.fill ? hexRgb(el.fill) : undefined, opacity: el.fill ? 0.35 : undefined, borderOpacity: 1 });
      } else if (el.t === "ellipse") {
        page.drawEllipse({ x: X(b.x + b.w / 2), y: Y(b.y + b.h / 2), xScale: Math.max(b.w / 2, 0.5), yScale: Math.max(b.h / 2, 0.5), borderColor: col, borderWidth: el.w, color: el.fill ? hexRgb(el.fill) : undefined, opacity: el.fill ? 0.35 : undefined, borderOpacity: 1 });
      } else {
        page.drawLine({ start: { x: X(el.x1), y: Y(el.y1) }, end: { x: X(el.x2), y: Y(el.y2) }, thickness: el.w, color: col, lineCap: LineCapStyle.Round });
        if (el.t === "arrow") page.drawSvgPath(arrowHead(el), { x: ox, y: oy, borderColor: col, borderWidth: el.w, borderLineCap: LineCapStyle.Round });
      }
      if ((el.t === "rect" || el.t === "ellipse") && el.text.trim()) {
        const lines = wrap(el.text, font, el.fs, Math.max(b.w - 8, 20));
        const lh = el.fs * 1.2;
        const top = b.y + (b.h - lh * lines.length) / 2;
        lines.forEach((ln, i) => {
          const wd = font.widthOfTextAtSize(ln, el.fs);
          page.drawText(ln, { x: X(b.x + (b.w - wd) / 2), y: Y(top + el.fs * 1.05 + lh * i), size: el.fs, font, color: col });
        });
      }
    }
  }
  return { bytes: await out.save(), skipped };
}
