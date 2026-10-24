//! Read-only view of the paper library's list (`[papers] list` in config.toml, by default Paper_Database.csv).
//!
//! This module only reads the file. The one place that writes it is "Add papers" (addpapers.rs), on the user's explicit request,
//! through the library's own `add_command` when one is set. Rows are identified by their Link (row numbers are not stable).

use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct Paper {
    /// Stable short id: hash of the Link (S. No is NOT stable).
    pub id: String,
    pub sno: u32,
    pub section: String,
    pub stage: String,
    pub role: String,
    pub title: String,
    pub topic: String,
    pub projects: String,
    pub link: String,
    pub est_hours: String,
    pub tier: String,
    pub notes: String,
    pub printed: bool,
    pub date_read: String,
    pub resources: String,
    /// Absolute path of the local PDF if the row has a Local Link (it may still be missing on disk).
    #[serde(skip)]
    pub local_path: Option<PathBuf>,
    pub arxiv: Option<String>,
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn id_for(link: &str) -> String {
    format!("{:010x}", fnv(link) & 0xff_ffff_ffff)
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// `file:///Users/x/My%20Papers/a.pdf` -> `/Users/x/My Papers/a.pdf`. None for anything that is not a file URL.
pub fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let b = rest.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    let s = String::from_utf8(out).ok()?;
    s.starts_with('/').then(|| PathBuf::from(s))
}

/// arXiv id from an abs/pdf link: https://arxiv.org/abs/2103.14030v2 -> 2103.14030 (also old style hep-th/9901001).
pub fn arxiv_id(link: &str) -> Option<String> {
    let l = link.trim();
    let rest = l.strip_prefix("https://arxiv.org/abs/").or_else(|| l.strip_prefix("http://arxiv.org/abs/")).or_else(|| l.strip_prefix("https://arxiv.org/pdf/"))?;
    let rest = rest.trim_end_matches(".pdf");
    let id = match rest.rfind('v') {
        Some(p) if rest[p + 1..].chars().all(|c| c.is_ascii_digit()) && p + 1 < rest.len() => &rest[..p],
        _ => rest,
    };
    (!id.is_empty()).then(|| id.to_string())
}

pub fn load(db: &Path) -> Result<Vec<Paper>> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(db).with_context(|| format!("cannot open {}", db.display()))?;
    let headers = rdr.headers()?.clone();
    let col = |n: &str| headers.iter().position(|h| h == n);
    let g = |rec: &csv::StringRecord, n: &str| col(n).and_then(|i| rec.get(i)).unwrap_or("").trim().to_string();
    let mut out = Vec::new();
    for rec in rdr.records() {
        let rec = rec?;
        let link = g(&rec, "Link");
        if link.is_empty() {
            continue;
        }
        let local = g(&rec, "Local Link");
        out.push(Paper {
            id: id_for(&link),
            sno: g(&rec, "S. No").parse().unwrap_or(0),
            section: g(&rec, "Section"),
            stage: g(&rec, "Stage"),
            role: g(&rec, "Role"),
            title: g(&rec, "Title"),
            topic: g(&rec, "DL_Topic"),
            projects: g(&rec, "Projects"),
            est_hours: g(&rec, "Est_Hours"),
            tier: g(&rec, "Tier"),
            notes: g(&rec, "Notes"),
            printed: g(&rec, "Printed").eq_ignore_ascii_case("yes"),
            date_read: g(&rec, "Date_Read"),
            resources: g(&rec, "Resources"),
            local_path: file_url_to_path(&local),
            arxiv: arxiv_id(&link),
            link,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct Bank {
    pub folder: String,
    pub pdf: String,
    pub link: String,
}

/// banks.csv: folder, pdf, link, title_in_db, matched_by. Returns link -> banks (a paper may have several).
pub fn load_banks(csv_path: &Path) -> HashMap<String, Vec<Bank>> {
    let mut m: HashMap<String, Vec<Bank>> = HashMap::new();
    let Ok(mut rdr) = csv::Reader::from_path(csv_path) else { return m };
    for rec in rdr.records().flatten() {
        let (folder, pdf, link) = (rec.get(0).unwrap_or(""), rec.get(1).unwrap_or(""), rec.get(2).unwrap_or(""));
        if !link.is_empty() && !pdf.is_empty() {
            m.entry(link.to_string()).or_default().push(Bank { folder: folder.into(), pdf: pdf.into(), link: link.into() });
        }
    }
    m
}

// ---------- inbox: papers downloaded by CourseShelf, kept apart from the library ----------

#[derive(Debug, Clone, Serialize)]
pub struct InboxRec {
    pub id: String,
    pub link: String,
    pub file: String,
    pub bytes: u64,
    pub pages: u32,
    pub when_unix: u64,
}

pub fn load_inbox(dir: &Path) -> Vec<InboxRec> {
    let Ok(mut rdr) = csv::Reader::from_path(dir.join("downloads.csv")) else { return vec![] };
    rdr.records()
        .flatten()
        .filter_map(|r| {
            Some(InboxRec {
                id: r.get(0)?.to_string(),
                link: r.get(1)?.to_string(),
                file: r.get(2)?.to_string(),
                bytes: r.get(3)?.parse().ok()?,
                pages: r.get(4)?.parse().unwrap_or(0),
                when_unix: r.get(5)?.parse().unwrap_or(0),
            })
        })
        .collect()
}

pub fn append_inbox(dir: &Path, rec: &InboxRec) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("downloads.csv");
    let new = !path.exists();
    let f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    let mut w = csv::WriterBuilder::new().has_headers(false).from_writer(f);
    if new {
        w.write_record(["id", "link", "file", "bytes", "pages", "when_unix"])?;
    }
    w.write_record([&rec.id, &rec.link, &rec.file, &rec.bytes.to_string(), &rec.pages.to_string(), &rec.when_unix.to_string()])?;
    w.flush()?;
    Ok(())
}

/// A download is accepted only if it really is a PDF: the library notes record sites that answer
/// 200 with an HTML challenge page, so the status code alone proves nothing.
pub fn check_pdf(head: &[u8], tail: &[u8], len: u64) -> std::result::Result<(), String> {
    if len < 20_000 {
        return Err(format!("only {len} bytes: too small to be a paper"));
    }
    if !head.starts_with(b"%PDF-") {
        let looks_html = head.iter().take(200).copied().collect::<Vec<u8>>();
        let t = String::from_utf8_lossy(&looks_html).to_ascii_lowercase();
        return Err(if t.contains("<html") || t.contains("<!doctype") { "the server sent a web page, not a PDF (blocked or paywalled)".into() } else { "file does not start with %PDF".into() });
    }
    if !tail.windows(5).any(|w| w == b"%%EOF") {
        return Err("PDF looks truncated (no end marker)".into());
    }
    Ok(())
}

/// Folder-safe name: `<arxiv id>_<title words>.pdf`
pub fn inbox_name(arxiv: Option<&str>, link: &str, title: &str) -> String {
    let base: String = arxiv.map(|a| a.replace('/', "_")).unwrap_or_else(|| id_for(link));
    let words: String = title
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .take(8)
        .collect::<Vec<_>>()
        .join("_");
    let words: String = words.chars().take(60).collect();
    if words.is_empty() { format!("{base}.pdf") } else { format!("{base}_{words}.pdf") }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pdf_checks() {
        use super::check_pdf;
        assert!(check_pdf(b"%PDF-1.5\n....", b"...%%EOF\n", 50_000).is_ok());
        assert!(check_pdf(b"<!DOCTYPE html><html>", b"</html>", 50_000).unwrap_err().contains("web page"));
        assert!(check_pdf(b"%PDF-1.5", b"no end", 50_000).unwrap_err().contains("truncated"));
        assert!(check_pdf(b"%PDF-1.5", b"%%EOF", 100).unwrap_err().contains("too small"));
    }

    #[test]
    fn names() {
        use super::inbox_name;
        assert_eq!(inbox_name(Some("1906.05478"), "x", "Robust and Interpretable Blind Image Denoising via Bias-Free CNNs: extra words here"), "1906.05478_Robust_and_Interpretable_Blind_Image_Denoising_via_Bias.pdf");
        assert_eq!(inbox_name(Some("hep-th/9901001"), "x", "A B"), "hep-th_9901001_A_B.pdf");
    }

    use super::*;

    #[test]
    fn file_urls() {
        assert_eq!(file_url_to_path("file:///a/b%20c/d.pdf"), Some(PathBuf::from("/a/b c/d.pdf")));
        assert_eq!(file_url_to_path("not in library"), None);
        assert_eq!(file_url_to_path("file://relative"), None);
        assert_eq!(file_url_to_path("file:///x/100%.pdf"), Some(PathBuf::from("/x/100%.pdf")));
    }

    #[test]
    fn arxiv_ids() {
        assert_eq!(arxiv_id("https://arxiv.org/abs/2103.14030"), Some("2103.14030".into()));
        assert_eq!(arxiv_id("https://arxiv.org/abs/2103.14030v2"), Some("2103.14030".into()));
        assert_eq!(arxiv_id("https://arxiv.org/pdf/1412.6980v9.pdf"), Some("1412.6980".into()));
        assert_eq!(arxiv_id("https://arxiv.org/abs/hep-th/9901001"), Some("hep-th/9901001".into()));
        assert_eq!(arxiv_id("https://doi.org/10.1037/h0042519"), None);
    }

    #[test]
    fn ids_are_stable_and_distinct() {
        assert_eq!(id_for("a"), id_for("a"));
        assert_ne!(id_for("a"), id_for("b"));
        assert_eq!(id_for("a").len(), 10);
    }
}
