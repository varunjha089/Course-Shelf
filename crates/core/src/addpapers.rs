//! Adding papers to the paper list (`[papers] list`): read what the user pasted (arXiv ids or links, DOIs), look the papers up
//! (arXiv API, Crossref), and write the new rows.
//!
//! Writing is the ONLY place CourseShelf changes the paper list, and only when the user presses Add. Two ways:
//! - `[papers] add_command = ["python3", "/path/to/add_papers.py"]`: the library's own tool does it (it knows the library's rules:
//!   ordering, renumbering, checks). CourseShelf writes the new papers to a JSON file and runs the command with that file as its
//!   last argument, in the library folder. Exit code 0 = added.
//! - no add_command: the rows are appended to the end of the list, after a backup copy in `<data folder>/papers/backups/`.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A paper as found online, plus what the user chose (topic, notes).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct NewPaper {
    pub arxiv: Option<String>,
    pub doi: Option<String>,
    pub title: String,
    pub authors: Vec<String>,
    pub year: String,
    /// Journal or conference (Crossref), or arXiv's journal_ref / comment.
    pub venue: String,
    pub link: String,
    pub summary: String,
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ref {
    Arxiv(String),
    Doi(String),
}

fn is_new_arxiv(s: &str) -> bool {
    // 2307.03172 or 0704.0001 (4 digits, dot, 4-5 digits)
    let b = s.as_bytes();
    let dot = s.find('.');
    matches!(dot, Some(4)) && b[..4].iter().all(u8::is_ascii_digit) && (s.len() == 9 || s.len() == 10) && b[5..].iter().all(u8::is_ascii_digit)
}

fn is_old_arxiv(s: &str) -> bool {
    // hep-th/9901001, math.GT/0309136
    match s.split_once('/') {
        Some((arch, num)) => {
            !arch.is_empty() && arch.chars().all(|c| c.is_ascii_alphabetic() || c == '-' || c == '.') && num.len() == 7 && num.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

fn strip_version(s: &str) -> &str {
    match s.rfind('v') {
        Some(p) if p > 0 && p + 1 < s.len() && s[p + 1..].chars().all(|c| c.is_ascii_digit()) => &s[..p],
        _ => s,
    }
}

/// What the user pasted -> (references understood, pieces not understood). Separators: whitespace, commas, semicolons.
/// Accepts 2307.03172, 2307.03172v2, arXiv:2307.03172, https://arxiv.org/abs/..., /pdf/... (.pdf), 10.xxxx/..., https://doi.org/10.xxxx/...
pub fn parse_refs(text: &str) -> (Vec<Ref>, Vec<String>) {
    let mut refs: Vec<Ref> = vec![];
    let mut bad = vec![];
    for raw in text.split(|c: char| c.is_whitespace() || c == ',' || c == ';') {
        let t = raw.trim().trim_matches(|c| c == '"' || c == '\'' || c == '<' || c == '>' || c == '(' || c == ')');
        if t.is_empty() {
            continue;
        }
        let lower = t.to_ascii_lowercase();
        let r = if let Some(i) = lower.find("doi.org/") {
            Some(Ref::Doi(t[i + 8..].trim_end_matches('.').to_string()))
        } else if lower.starts_with("10.") && t.contains('/') {
            Some(Ref::Doi(t.trim_end_matches('.').to_string()))
        } else {
            let mut s = t;
            for p in ["arxiv.org/abs/", "arxiv.org/pdf/"] {
                if let Some(i) = lower.find(p) {
                    s = &t[i + p.len()..];
                }
            }
            if s.len() > 6 && s[..6].eq_ignore_ascii_case("arxiv:") {
                s = &s[6..];
            }
            let s = strip_version(s.trim_end_matches(".pdf").trim_end_matches('/'));
            (is_new_arxiv(s) || is_old_arxiv(s)).then(|| Ref::Arxiv(s.to_string()))
        };
        match r {
            Some(r) if !refs.contains(&r) => refs.push(r),
            Some(_) => {}
            None => bad.push(t.to_string()),
        }
    }
    (refs, bad)
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#39;", "'").replace("&amp;", "&")
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text of the first <tag>...</tag> in `s` (attributes allowed).
fn tag<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let open = s.find(&format!("<{name}"))?;
    let start = open + s[open..].find('>')? + 1;
    let end = start + s[start..].find(&format!("</{name}>"))?;
    Some(&s[start..end])
}

fn all_tags<'a>(mut s: &'a str, name: &str) -> Vec<&'a str> {
    let mut out = vec![];
    while let Some(t) = tag(s, name) {
        out.push(t);
        let close = format!("</{name}>");
        match s.find(&close) {
            Some(i) => s = &s[i + close.len()..],
            None => break,
        }
    }
    out
}

/// Parse an arXiv API (Atom) answer. Entries that are errors (unknown id) are left out.
pub fn parse_arxiv_atom(xml: &str) -> Vec<NewPaper> {
    let mut out = vec![];
    for entry in xml.split("<entry>").skip(1) {
        let entry = entry.split("</entry>").next().unwrap_or("");
        let id_url = tag(entry, "id").unwrap_or("").trim();
        let Some(rest) = id_url.split("/abs/").nth(1) else { continue };
        let id = strip_version(rest).to_string();
        let title = squash(&unescape(tag(entry, "title").unwrap_or("")));
        if title.is_empty() || title == "Error" {
            continue;
        }
        let authors = all_tags(entry, "author").iter().filter_map(|a| tag(a, "name")).map(|n| squash(&unescape(n))).collect();
        let year = tag(entry, "published").map(|p| p.trim().chars().take(4).collect()).unwrap_or_default();
        let venue = tag(entry, "arxiv:journal_ref").or_else(|| tag(entry, "arxiv:comment")).map(|v| squash(&unescape(v))).unwrap_or_default();
        let doi = tag(entry, "arxiv:doi").map(|d| d.trim().to_string());
        out.push(NewPaper {
            link: format!("https://arxiv.org/abs/{id}"),
            arxiv: Some(id),
            doi,
            title,
            authors,
            year,
            venue,
            summary: squash(&unescape(tag(entry, "summary").unwrap_or(""))),
            ..Default::default()
        });
    }
    out
}

/// Parse a Crossref `/works/<doi>` answer.
pub fn parse_crossref(json: &str, doi: &str) -> Result<NewPaper> {
    let v: serde_json::Value = serde_json::from_str(json).context("Crossref did not answer with JSON")?;
    let m = &v["message"];
    let first = |k: &str| m[k].as_array().and_then(|a| a.first()).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let title = squash(&first("title"));
    if title.is_empty() {
        bail!("Crossref has no title for {doi}");
    }
    let authors = m["author"]
        .as_array()
        .map(|a| a.iter().map(|p| squash(&format!("{} {}", p["given"].as_str().unwrap_or(""), p["family"].as_str().unwrap_or("")))).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let year = ["published-print", "published-online", "issued", "created"]
        .iter()
        .find_map(|k| m[*k]["date-parts"][0][0].as_i64())
        .map(|y| y.to_string())
        .unwrap_or_default();
    Ok(NewPaper {
        arxiv: None,
        doi: Some(doi.to_string()),
        title,
        authors,
        year,
        venue: first("container-title"),
        link: format!("https://doi.org/{doi}"),
        summary: String::new(),
        ..Default::default()
    })
}

fn curl(url: &str) -> Result<String> {
    let out = crate::tools::command("curl")
        .args(["-sSL", "--fail", "--max-time", "60", "-A", "CourseShelf/0.1 (personal research library; adding papers)", url])
        .output()
        .map_err(|e| anyhow!("could not run curl: {e}"))?;
    if !out.status.success() {
        bail!("{} ({})", String::from_utf8_lossy(&out.stderr).trim(), url);
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Look the references up online: arXiv ids in one batch, DOIs one by one. Returns the papers found and one message per failure.
pub fn lookup(refs: &[Ref]) -> (Vec<NewPaper>, Vec<String>) {
    let mut found = vec![];
    let mut errors = vec![];
    let ids: Vec<&str> = refs.iter().filter_map(|r| if let Ref::Arxiv(a) = r { Some(a.as_str()) } else { None }).collect();
    if !ids.is_empty() {
        match curl(&format!("https://export.arxiv.org/api/query?id_list={}&max_results={}", ids.join(","), ids.len() + 5)) {
            Ok(xml) => {
                let got = parse_arxiv_atom(&xml);
                for a in &ids {
                    if !got.iter().any(|p| p.arxiv.as_deref() == Some(*a)) {
                        errors.push(format!("{a}: not found on arXiv"));
                    }
                }
                found.extend(got);
            }
            Err(e) => errors.push(format!("arXiv could not be reached: {e}")),
        }
    }
    for r in refs {
        if let Ref::Doi(d) = r {
            match curl(&format!("https://api.crossref.org/works/{d}")).and_then(|j| parse_crossref(&j, d)) {
                Ok(p) => found.push(p),
                Err(e) => errors.push(format!("{d}: {e:#}")),
            }
        }
    }
    (found, errors)
}

/// Append the papers to the end of a CSV paper list (the case without add_command). Columns are filled by name when the list has
/// them: S. No (largest + 1), Title, Link, Notes, DL_Topic, Local Link ("not in library"). A backup copy is made first.
/// Returns the backup's path.
/// The column headings of a new starter list (the columns "Add papers" fills, read back by `papers::load`).
pub const STARTER_LIST_HEADER: &str = "S. No,Title,DL_Topic,Link,Local Link,Notes\n";

/// Make an empty starter list (headings only) when there is none yet. Only for `PapersPaths::starter`, never for a configured library.
pub fn start_list(list_csv: &Path) -> Result<bool> {
    if list_csv.exists() {
        return Ok(false);
    }
    if let Some(dir) = list_csv.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(list_csv, STARTER_LIST_HEADER).with_context(|| format!("cannot write {}", list_csv.display()))?;
    Ok(true)
}

pub fn append_to_list(list_csv: &Path, papers: &[NewPaper], backup_dir: &Path) -> Result<PathBuf> {
    if papers.is_empty() {
        bail!("no paper to add");
    }
    let original = std::fs::read(list_csv).with_context(|| format!("cannot read {}", list_csv.display()))?;
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(original.as_slice());
    let headers = rdr.headers()?.clone();
    let col = |n: &str| headers.iter().position(|h| h == n);
    let (c_title, c_link) = (col("Title").ok_or_else(|| anyhow!("the list has no Title column"))?, col("Link").ok_or_else(|| anyhow!("the list has no Link column"))?);
    let mut max_sno = 0u64;
    let mut links = std::collections::HashSet::new();
    for rec in rdr.records() {
        let rec = rec?;
        if let Some(n) = col("S. No").and_then(|i| rec.get(i)).and_then(|s| s.trim().parse::<u64>().ok()) {
            max_sno = max_sno.max(n);
        }
        links.insert(rec.get(c_link).unwrap_or("").trim().to_string());
    }
    for p in papers {
        if links.contains(&p.link) {
            bail!("{} is already in the list", p.link);
        }
    }
    std::fs::create_dir_all(backup_dir)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let name = list_csv.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "list.csv".into());
    let backup = backup_dir.join(format!("{name}.{stamp}.bak"));
    std::fs::write(&backup, &original)?;
    let mut buf = original.clone();
    if !buf.ends_with(b"\n") {
        buf.push(b'\n');
    }
    {
        let mut w = csv::WriterBuilder::new().has_headers(false).from_writer(&mut buf);
        for p in papers {
            max_sno += 1;
            let mut row = vec![String::new(); headers.len()];
            row[c_title] = p.title.clone();
            row[c_link] = p.link.clone();
            let mut set = |n: &str, v: String| {
                if let Some(i) = col(n) {
                    row[i] = v;
                }
            };
            set("S. No", max_sno.to_string());
            set("Notes", p.notes.clone());
            set("DL_Topic", p.topic.clone());
            set("Local Link", "not in library".into());
            w.write_record(&row)?;
        }
        w.flush()?;
    }
    let tmp = list_csv.with_extension("csv.tmp");
    std::fs::write(&tmp, &buf)?;
    std::fs::rename(&tmp, list_csv)?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn understands_what_people_paste() {
        let (r, bad) = parse_refs(
            "https://arxiv.org/abs/2307.03172, 2610.20341v1\narXiv:1905.11946 https://arxiv.org/pdf/2005.11401v4.pdf hep-th/9901001 \
             https://doi.org/10.1038/323533a0 10.1007/BF02551274. hello 2307.03172",
        );
        assert_eq!(
            r,
            vec![
                Ref::Arxiv("2307.03172".into()),
                Ref::Arxiv("2610.20341".into()),
                Ref::Arxiv("1905.11946".into()),
                Ref::Arxiv("2005.11401".into()),
                Ref::Arxiv("hep-th/9901001".into()),
                Ref::Doi("10.1038/323533a0".into()),
                Ref::Doi("10.1007/BF02551274".into()),
            ]
        );
        assert_eq!(bad, vec!["hello".to_string()]); // the repeated id is not listed twice
    }

    #[test]
    fn reads_an_arxiv_answer() {
        let xml = r#"<feed><entry><id>http://arxiv.org/abs/2307.03172v3</id><published>2023-07-06T17:54:11Z</published>
            <title>Lost in the Middle: How Language
              Models Use Long Contexts</title><summary> We analyze &amp; find </summary>
            <author><name>Nelson F. Liu</name></author><author><name>Kevin Lin</name></author>
            <arxiv:comment xmlns:arxiv="http://arxiv.org/schemas/atom">TACL 2023</arxiv:comment></entry>
            <entry><id>http://arxiv.org/api/errors#incorrect_id_format_for_x</id><title>Error</title></entry></feed>"#;
        let p = parse_arxiv_atom(xml);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].arxiv.as_deref(), Some("2307.03172"));
        assert_eq!(p[0].title, "Lost in the Middle: How Language Models Use Long Contexts");
        assert_eq!(p[0].authors, vec!["Nelson F. Liu", "Kevin Lin"]);
        assert_eq!((p[0].year.as_str(), p[0].venue.as_str(), p[0].summary.as_str()), ("2023", "TACL 2023", "We analyze & find"));
        assert_eq!(p[0].link, "https://arxiv.org/abs/2307.03172");
    }

    #[test]
    fn reads_a_crossref_answer() {
        let j = r#"{"message":{"title":["Learning representations by back-propagating errors"],"author":[{"given":"David E.","family":"Rumelhart"}],
                   "issued":{"date-parts":[[1986,10]]},"container-title":["Nature"]}}"#;
        let p = parse_crossref(j, "10.1038/323533a0").unwrap();
        assert_eq!((p.title.as_str(), p.year.as_str(), p.venue.as_str()), ("Learning representations by back-propagating errors", "1986", "Nature"));
        assert_eq!(p.authors, vec!["David E. Rumelhart"]);
        assert_eq!(p.link, "https://doi.org/10.1038/323533a0");
    }

    #[test]
    fn a_starter_list_is_made_once_then_appended_to() {
        let d = std::env::temp_dir().join(format!("cs_starter_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let list = d.join("papers").join("papers.csv");
        assert!(start_list(&list).unwrap());
        assert!(crate::papers::load(&list).unwrap().is_empty());
        let p = NewPaper { title: "A paper".into(), link: "https://arxiv.org/abs/2307.03172".into(), topic: "T".into(), ..Default::default() };
        append_to_list(&list, &[p], &d.join("backups")).unwrap();
        assert!(!start_list(&list).unwrap(), "an existing list is never replaced");
        let got = crate::papers::load(&list).unwrap();
        assert_eq!((got.len(), got[0].sno, got[0].title.as_str(), got[0].topic.as_str(), got[0].arxiv.as_deref()), (1, 1, "A paper", "T", Some("2307.03172")));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn appends_with_a_backup_and_refuses_duplicates() {
        let d = std::env::temp_dir().join(format!("cs_add_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let list = d.join("list.csv");
        std::fs::write(&list, "S. No,Title,DL_Topic,Link,Local Link,Notes\n1,A,T,https://arxiv.org/abs/1,file:///x.pdf,\"a, b\"\n").unwrap();
        let p = NewPaper { title: "New, with comma".into(), link: "https://arxiv.org/abs/2".into(), topic: "RAG".into(), notes: "n".into(), ..Default::default() };
        let backup = append_to_list(&list, &[p.clone()], &d.join("backups")).unwrap();
        assert!(backup.exists());
        let t = std::fs::read_to_string(&list).unwrap();
        assert!(t.ends_with("2,\"New, with comma\",RAG,https://arxiv.org/abs/2,not in library,n\n"), "{t}");
        assert!(append_to_list(&list, &[p], &d.join("backups")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
