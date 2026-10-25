export interface Lecture {
  index: number;
  key: string;
  title: string;
  file: string;
  duration: number | null;
  width: number | null;
  height: number | null;
  vcodec: string | null;
  size_bytes: number;
  subs: string[];
  position: number;
  done: boolean;
  last_watched: number;
}

export interface CopyInfo {
  drive: string;
  location: string;
  connected: boolean;
  is_mac: boolean;
  overridden: boolean;
  lectures: number;
  size_bytes: number;
}

export interface MacInfo {
  drive: string;
  path: string;
  used_bytes: number;
  max_bytes: number | null;
  free_bytes: number | null;
}

export interface Course {
  drive: string;
  course: string;
  title: string;
  listed: boolean;
  enabled: boolean;
  priority: number | null;
  note: string;
  playlist_url: string;
  items: string;
  connected: boolean;
  speed: number;
  done_count: number;
  location: string;
  overridden: boolean;
  copies: CopyInfo[];
  lectures: Lecture[];
}

export interface Drive {
  name: string;
  path: string;
  connected: boolean;
  free_bytes: number | null;
  total_bytes: number | null;
  budget_gb: number | null;
}

export interface Recent {
  drive: string;
  course: string;
  course_title: string;
  key: string;
  title: string;
  file: string;
  position: number;
  duration: number;
  last_watched: number;
}

export interface State {
  drives: Drive[];
  courses: Course[];
  recent: Recent[];
  mac: MacInfo | null;
  error: string | null;
}

export async function getState(): Promise<State> {
  const r = await fetch("/api/state", { cache: "no-store" });
  if (r.status === 401) throw new Error("Not authorised. Open CourseShelf from its app window (the link carries a one-time key).");
  if (!r.ok) throw new Error(`Server said ${r.status}`);
  return r.json();
}

async function post(path: string, body: unknown, keepalive = false) {
  const r = await fetch(path, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body), keepalive });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

export const saveProgress = (drive: string, course: string, key: string, position: number, duration: number, keepalive = false): Promise<{ done: boolean }> =>
  post("/api/progress", { drive, course, key, position, duration }, keepalive);

export const setDone = (drive: string, course: string, key: string, done: boolean) => post("/api/done", { drive, course, key, done });

export const setSpeed = (drive: string, course: string, speed: number) => post("/api/speed", { drive, course, speed });

export const mediaUrl = (drive: string, course: string, file: string) =>
  `/media/${encodeURIComponent(drive)}/${course.split("/").map(encodeURIComponent).join("/")}/${encodeURIComponent(file)}`;

export async function getSubs(drive: string, course: string, file: string): Promise<string | null> {
  const q = new URLSearchParams({ drive, course, file });
  const r = await fetch(`/api/subs?${q}`);
  return r.ok ? r.text() : null;
}

export function fmtTime(s: number | null | undefined): string {
  if (s == null || !isFinite(s) || s < 0) return "–";
  const t = Math.round(s);
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const sec = t % 60;
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}` : `${m}:${String(sec).padStart(2, "0")}`;
}

export function fmtHours(s: number): string {
  const m = Math.round(s / 60);
  return m >= 60 ? `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")} m` : `${m} m`;
}

export function fmtBytes(n: number | null | undefined): string {
  if (n == null) return "–";
  const gb = n / 1e9;
  return gb >= 100 ? `${gb.toFixed(0)} GB` : gb >= 1 ? `${gb.toFixed(1)} GB` : `${(n / 1e6).toFixed(0)} MB`;
}

export interface Settings {
  font: number;
  theme: "auto" | "light" | "dark";
  autonext: boolean;
}

export const DEFAULT_SETTINGS: Settings = { font: 19, theme: "auto", autonext: true };

/** Display settings live in a file next to config.toml, so they survive restarts (the app's port changes every launch). */
export async function getSettings(): Promise<Settings> {
  try {
    const r = await fetch("/api/settings", { cache: "no-store" });
    if (!r.ok) return DEFAULT_SETTINGS;
    return { ...DEFAULT_SETTINGS, ...(await r.json()) };
  } catch {
    return DEFAULT_SETTINGS;
  }
}

export const saveSettings = (patch: Partial<Settings>) => post("/api/settings", patch).catch(() => {});

// ---------- papers ----------

export interface Bank {
  folder: string;
  pdf: string;
  link: string;
}

export interface Paper {
  id: string;
  sno: number;
  section: string;
  stage: string;
  role: string;
  title: string;
  topic: string;
  projects: string;
  link: string;
  est_hours: string;
  tier: string;
  notes: string;
  printed: boolean;
  date_read: string;
  resources: string;
  arxiv: string | null;
  held: boolean;
  missing_local: boolean;
  banks: Bank[];
  downloadable: boolean;
  inbox: { id: string; file: string; bytes: number; pages: number } | null;
}

export interface PapersState {
  available: boolean;
  error: string | null;
  library: string;
  list: string;
  starter: boolean;
  papers: Paper[];
}

export async function getPapers(): Promise<PapersState> {
  const r = await fetch("/api/papers", { cache: "no-store" });
  if (!r.ok) throw new Error(`Server said ${r.status}`);
  return r.json();
}

export const paperPdfUrl = (id: string) => `/pdf/paper/${encodeURIComponent(id)}`;
export const bankPdfUrl = (folder: string, pdf: string) => `/pdf/bank/${encodeURIComponent(folder)}/${encodeURIComponent(pdf)}`;

export const openExternal = (url: string) => post("/api/open", { kind: "external", url }).catch(() => {});
export const openPaper = (id: string, reveal: boolean) => post("/api/open", { kind: "paper", id, reveal }).catch(() => {});
export const openBank = (folder: string, pdf: string, reveal: boolean) => post("/api/open", { kind: "bank", folder, pdf, reveal }).catch(() => {});

export const downloadPaper = (id: string): Promise<{ ok: boolean; file: string; bytes: number; pages: number }> => post("/api/papers/download", { id });
export const inboxPdfUrl = (id: string) => `/pdf/inbox/${encodeURIComponent(id)}`;
export const openInbox = (id: string, reveal: boolean) => post("/api/open", { kind: "inbox", id, reveal }).catch(() => {});

// ---------- downloads, existing folders, moving ----------

export interface ManageCourse {
  course: string;
  title: string;
  drive: string;
  listed: boolean;
  size_bytes: number;
  enabled: boolean;
  playlist_url: string;
  items: string;
  note: string;
  drive_connected: boolean;
  location: string;
  overridden: boolean;
  folder_exists: boolean;
  on_disk: number;
  free_bytes: number | null;
}

export interface DriveOut {
  name: string;
  path: string;
  connected: boolean;
  free_bytes: number | null;
  total_bytes: number | null;
  budget_gb: number | null;
  is_mac: boolean;
}

export interface ManageState {
  courses: ManageCourse[];
  drives: DriveOut[];
  ytdlp: string | null;
  home: string;
  error: string | null;
}

export interface Job {
  id: number;
  kind: "download" | "move" | "single" | "collection" | "addpapers";
  course: string;
  drive: string;
  title: string;
  status: "queued" | "running" | "done" | "failed" | "cancelled";
  started: number;
  finished: number;
  total: number;
  done: number;
  current_index: number;
  current_title: string;
  percent: number;
  speed: string;
  eta: string;
  bytes_done: number;
  bytes_total: number;
  failed: string[];
  warnings: string[];
  message: string;
  log: string[];
}

export interface DlPlan {
  playlist_title: string;
  total: number;
  wanted: number;
  have: number;
  missing: number[];
  est_bytes: number;
  free_bytes: number | null;
  fits: boolean;
  drive_connected: boolean;
  title_warnings: string[];
  note: string;
  location: string;
}

export interface Proposal {
  file: string;
  index: number | null;
  entry_title: string | null;
  new_name: string | null;
  score: number;
  status: "ok" | "rename" | "unmatched";
}

export interface MovePlan {
  from: string;
  to: string;
  files: number;
  total_bytes: number;
  same_fs: boolean;
  already_there: number;
  free_bytes: number | null;
  ok: boolean;
  reason: string;
}

export interface BrowseOut {
  path: string;
  parent: string | null;
  videos_here: number;
  dirs: { name: string; path: string; videos: number }[];
  home: string | null;
}

export async function getJson<T>(path: string): Promise<T> {
  const r = await fetch(path, { cache: "no-store" });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}

export const getManage = () => getJson<ManageState>("/api/manage");
export const getJobs = () => getJson<{ jobs: Job[] }>("/api/manage/jobs").then((x) => x.jobs);
export const planDownload = (course: string, refresh = false, dest_root = "") => post("/api/manage/plan", { course, refresh, dest_root }) as Promise<DlPlan>;
export const startDownload = (course: string, force_titles = false, dest_root = "") => post("/api/manage/download", { course, force_titles, dest_root }) as Promise<{ job: number }>;
export const cancelJob = (job: number) => post("/api/manage/cancel", { job });
export const setLocation = (course: string, path: string) => post("/api/manage/location", { course, path }) as Promise<{ location: string; videos: number }>;
export const clearLocation = (course: string) => post("/api/manage/location/clear", { course });
export const adoptPreview = (course: string, refresh = false) =>
  post("/api/manage/adopt/preview", { course, refresh }) as Promise<{ playlist_title: string; playlist_total: number; proposals: Proposal[]; positions_without_file: number }>;
export const adoptApply = (course: string, renames: [string, string][]) => post("/api/manage/adopt/apply", { course, renames }) as Promise<{ renamed: number }>;
export const planMove = (course: string, to_drive: string, keep_original: boolean, from_drive?: string) => post("/api/manage/move/plan", { course, to_drive, keep_original, from_drive }) as Promise<MovePlan>;
export const startMove = (course: string, to_drive: string, keep_original: boolean, from_drive?: string) => post("/api/manage/move/start", { course, to_drive, keep_original, from_drive }) as Promise<{ job: number }>;
export const browse = (path?: string) => getJson<BrowseOut>(`/api/manage/browse${path ? `?path=${encodeURIComponent(path)}` : ""}`);

export const setDrive = (name: string, path: string, create = false) => post("/api/manage/drive", { name, path, create }) as Promise<{ ok: boolean; path: string }>;

export const macCopy = (course: string) => post("/api/manage/mac/copy", { course }) as Promise<{ job: number }>;
export const macRemove = (course: string) => post("/api/manage/mac/remove", { course }) as Promise<{ removed: number; freed_bytes: number; kept_copy_on: string }>;

// ---------- books and PDF annotations ----------

export interface BookOut { id: string; name: string; folder: string; rel: string; size: number; bookmarked: boolean }
export interface CollPaper { paper_id: string; title: string; pages: number; start: number }
export interface Collection { id: string; title: string; papers: CollPaper[]; bookmarked?: boolean }
export interface BooksOut { folders: { path: string; connected: boolean; count: number }[]; books: BookOut[]; collections: Collection[] }
export const getCollections = () => getJson<{ collections: Collection[] }>("/api/collections").then((x) => x.collections);
export const createCollection = (title: string, paper_ids: string[]) => post("/api/collections/create", { title, paper_ids }) as Promise<{ job: number }>;
export const addToCollection = (id: string, paper_ids: string[]) => post("/api/collections/add", { id, paper_ids }) as Promise<{ job: number }>;
export const reorderCollection = (id: string, order: string[]) => post("/api/collections/reorder", { id, order });
export const renameCollection = (id: string, title: string) => post("/api/collections/rename", { id, title });
export const deleteCollection = (id: string) => post("/api/collections/delete", { id });
export const collectionPdfUrl = (id: string) => `/pdf/collection/${encodeURIComponent(id)}`;
export const openCollection = (id: string, reveal: boolean) => post("/api/open", { kind: "collection", id, reveal }).catch(() => {});
export const getBooks = () => getJson<BooksOut>("/api/books");
export const setBookmark = (id: string, on: boolean) => post("/api/books/bookmark", { id, on });
export const bookPdfUrl = (id: string) => `/pdf/book/${encodeURIComponent(id)}`;
export const openBook = (id: string, reveal: boolean) => post("/api/open", { kind: "book", id, reveal }).catch(() => {});

export const annotLoad = (kind: string, ref: string) => post("/api/annot/load", { kind, ref }) as Promise<{ key: string; doc: unknown; file: string }>;
export const annotSave = (key: string, doc: unknown) => post("/api/annot/save", { key, doc }) as Promise<{ ok: boolean }>;
export const annotImageUrl = (key: string, file: string) => `/api/annot/image/${key}/${encodeURIComponent(file)}`;
export async function annotUploadImage(key: string, blob: Blob, ext: "png" | "jpg"): Promise<string> {
  const r = await fetch(`/api/annot/image/${key}?ext=${ext}`, { method: "POST", body: blob });
  if (!r.ok) throw new Error(await r.text());
  return (await r.json()).file as string;
}
export async function annotExport(name: string, bytes: Uint8Array): Promise<{ file: string; path: string }> {
  const r = await fetch(`/api/annot/export?name=${encodeURIComponent(name)}`, { method: "POST", body: bytes as BodyInit });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}
export const annotReveal = (file: string) => post("/api/annot/reveal", { file }).catch(() => {});

// ---- single videos (pasted YouTube links) ----
export interface Single { id: string; kind: "video" | "playlist"; yt_id: string; url: string; title: string; uploader: string; duration: number | null; count: number; added: number; status: "saved" | "downloaded"; dest: string | null; files: string[]; downloaded_at: number }
export const getSingles = () => getJson<{ items: Single[]; last_dest: string | null }>("/api/singles");
export const addSingle = (url: string) => post("/api/singles/add", { url }) as Promise<Single>;
export const removeSingle = (id: string) => post("/api/singles/remove", { id });
export const downloadSingle = (id: string, dest: string, create = false) => post("/api/singles/download", { id, dest, create }) as Promise<{ job: number }>;
export const revealSingle = (id: string) => post("/api/open", { kind: "single", id, reveal: true }).catch(() => {});

// ---------- adding papers to the paper list ----------
export interface NewPaper { arxiv: string | null; doi: string | null; title: string; authors: string[]; year: string; venue: string; link: string; summary: string; topic: string; notes: string }
export interface LookedUp { paper: NewPaper; existing: { id: string; sno: number; title: string } | null }
export interface LookupOut { found: LookedUp[]; not_understood: string[]; errors: string[]; topics: string[]; add_command: string | null }
export const lookupPapers = (text: string) => post("/api/papers/lookup", { text }) as Promise<LookupOut>;
export const addPapers = (papers: NewPaper[]) => post("/api/papers/add", { papers }) as Promise<{ job: number }>;
