import { useEffect, useState } from "react";
import { Course } from "./api";

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
export const courseHref = (c: { drive: string; course: string }) => `#/c/${encodeURIComponent(c.drive)}/${enc(c.course)}`;
export const playHref = (c: { drive: string; course: string }, file: string) => `#/p/${encodeURIComponent(c.drive)}/${enc(c.course)}/${encodeURIComponent(file)}`;

export function useRoute(): Route {
  const [r, setR] = useState<Route>(() => parseHash(location.hash));
  useEffect(() => {
    const f = () => setR(parseHash(location.hash));
    addEventListener("hashchange", f);
    return () => removeEventListener("hashchange", f);
  }, []);
  return r;
}

export const keyOf = (c: { drive: string; course: string }) => `${c.drive}\u0000${c.course}`;
export const courseSeconds = (c: Course) => c.lectures.reduce((a, l) => a + (l.duration ?? 0), 0);
