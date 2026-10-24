//! Matching files you downloaded by hand to playlist positions, so `run` does not fetch them again
//! and the app can number them. Pure and previewable: nothing is renamed until `apply`.

use crate::library;
use anyhow::{bail, Result};
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct Proposal {
    pub file: String,
    pub index: Option<u32>,
    pub entry_title: Option<String>,
    pub new_name: Option<String>,
    pub score: f32,
    /// "ok" (already numbered correctly), "rename", or "unmatched"
    pub status: String,
}

fn tokens(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        // words that appear in nearly every lecture title and carry no information
        .filter(|t| !matches!(*t, "lecture" | "lec" | "part" | "the" | "and" | "of" | "to" | "in" | "a" | "an"))
        .map(|t| t.trim_start_matches('0').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let i = a.intersection(b).count() as f32;
    i / (a.len() + b.len()) as f32 * 2.0 // Dice: 1.0 when identical
}

fn sidecar_stems(dir: &Path, stem: &str) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.starts_with(stem)).collect())
        .unwrap_or_default()
}

/// `entries`: (playlist position, title). Each file is matched to at most one position and vice versa.
pub fn propose(dir: &Path, entries: &[(u32, String)]) -> Vec<Proposal> {
    let lectures = library::scan_folder(dir);
    let etoks: Vec<HashSet<String>> = entries.iter().map(|e| tokens(&e.1)).collect();
    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for (li, l) in lectures.iter().enumerate() {
        let stem = Path::new(&l.file).file_stem().unwrap().to_string_lossy().to_string();
        let (_, title_part) = library::split_prefix(&stem);
        let lt = tokens(&title_part);
        for (ei, et) in etoks.iter().enumerate() {
            let mut sc = jaccard(&lt, et);
            if l.index > 0 && l.index == entries[ei].0 {
                sc += 0.15; // the file already carries this position
            }
            if sc >= 0.5 {
                pairs.push((sc, li, ei));
            }
        }
    }
    pairs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut used_l = HashSet::new();
    let mut used_e = HashSet::new();
    let mut chosen: Vec<Option<(usize, f32)>> = vec![None; lectures.len()];
    for (sc, li, ei) in pairs {
        if used_l.contains(&li) || used_e.contains(&ei) {
            continue;
        }
        used_l.insert(li);
        used_e.insert(ei);
        chosen[li] = Some((ei, sc));
    }
    lectures
        .iter()
        .enumerate()
        .map(|(li, l)| match chosen[li] {
            Some((ei, sc)) => {
                let (idx, etitle) = (&entries[ei].0, &entries[ei].1);
                if l.index == *idx {
                    Proposal { file: l.file.clone(), index: Some(*idx), entry_title: Some(etitle.clone()), new_name: None, score: sc.min(1.0), status: "ok".into() }
                } else {
                    let stem = Path::new(&l.file).file_stem().unwrap().to_string_lossy().to_string();
                    let ext = Path::new(&l.file).extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
                    let (_, title_part) = library::split_prefix(&stem);
                    Proposal { file: l.file.clone(), index: Some(*idx), entry_title: Some(etitle.clone()), new_name: Some(format!("{idx:03} - {title_part}.{ext}")), score: sc.min(1.0), status: "rename".into() }
                }
            }
            None => Proposal { file: l.file.clone(), index: None, entry_title: None, new_name: None, score: 0.0, status: "unmatched".into() },
        })
        .collect()
}

/// Rename the approved files (and their .info.json / .vtt / thumbnails). Returns (old stem, new stem) pairs
/// so progress keyed by file name can follow. Refuses to overwrite anything.
pub fn apply(dir: &Path, renames: &[(String, String)]) -> Result<Vec<(String, String)>> {
    let stem = |f: &str| Path::new(f).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    // check everything first so a failure leaves the folder untouched
    let mut plan: Vec<(std::path::PathBuf, std::path::PathBuf)> = Vec::new();
    for (from, to) in renames {
        if from.contains('/') || to.contains('/') || to.starts_with('.') || from.starts_with('.') {
            bail!("unsafe file name: {from} -> {to}");
        }
        let (fs_, ts) = (stem(from), stem(to));
        if !dir.join(from).is_file() {
            bail!("{from} is not in the folder any more");
        }
        for name in sidecar_stems(dir, &fs_) {
            let rest = &name[fs_.len()..];
            if !rest.is_empty() && !rest.starts_with('.') {
                continue; // a different file that merely starts with the same words
            }
            let target = dir.join(format!("{ts}{rest}"));
            if target.exists() {
                bail!("{} already exists; nothing was renamed", target.file_name().unwrap().to_string_lossy());
            }
            plan.push((dir.join(&name), target));
        }
    }
    let mut targets = HashSet::new();
    for (_, t) in &plan {
        if !targets.insert(t.clone()) {
            bail!("two files would get the same name {}", t.display());
        }
    }
    for (a, b) in &plan {
        std::fs::rename(a, b)?;
    }
    Ok(renames.iter().map(|(f, t)| (stem(f), stem(t))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("cs_adopt_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn entries() -> Vec<(u32, String)> {
        vec![
            (1, "The Geometry of Linear Equations".into()),
            (2, "Elimination with Matrices".into()),
            (3, "Multiplication and Inverse Matrices".into()),
        ]
    }

    #[test]
    fn matches_by_title_and_renames_with_sidecars() {
        let d = dir("a");
        for f in ["Lecture 1 - The Geometry of Linear Equations.mp4", "Lecture 1 - The Geometry of Linear Equations.info.json", "elimination with matrices.mp4", "random notes.mp4"] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        let p = propose(&d, &entries());
        let by = |f: &str| p.iter().find(|x| x.file == f).unwrap();
        assert_eq!(by("Lecture 1 - The Geometry of Linear Equations.mp4").status, "rename");
        assert_eq!(by("Lecture 1 - The Geometry of Linear Equations.mp4").new_name.as_deref(), Some("001 - Lecture 1 - The Geometry of Linear Equations.mp4"));
        assert_eq!(by("elimination with matrices.mp4").index, Some(2));
        assert_eq!(by("random notes.mp4").status, "unmatched");
        let r = apply(&d, &[("elimination with matrices.mp4".into(), "002 - elimination with matrices.mp4".into())]).unwrap();
        assert_eq!(r[0].1, "002 - elimination with matrices");
        assert!(d.join("002 - elimination with matrices.mp4").is_file());
        // sidecar followed
        let r = apply(&d, &[("Lecture 1 - The Geometry of Linear Equations.mp4".into(), "001 - Lecture 1 - The Geometry of Linear Equations.mp4".into())]).unwrap();
        assert!(!r.is_empty());
        assert!(d.join("001 - Lecture 1 - The Geometry of Linear Equations.info.json").is_file());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn already_numbered_is_ok_and_no_overwrite() {
        let d = dir("b");
        std::fs::write(d.join("02 - Elimination with Matrices.mp4"), b"x").unwrap();
        std::fs::write(d.join("002 - taken.mp4"), b"x").unwrap();
        let p = propose(&d, &entries());
        assert_eq!(p.iter().find(|x| x.file.starts_with("02 -")).unwrap().status, "ok");
        assert!(apply(&d, &[("02 - Elimination with Matrices.mp4".into(), "002 - taken.mp4".into())]).is_err());
        assert!(d.join("02 - Elimination with Matrices.mp4").is_file());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn two_files_never_claim_one_position() {
        let d = dir("c");
        std::fs::write(d.join("Elimination with Matrices.mp4"), b"x").unwrap();
        std::fs::write(d.join("Elimination with Matrices copy.mp4"), b"x").unwrap();
        let p = propose(&d, &entries());
        assert_eq!(p.iter().filter(|x| x.index == Some(2)).count(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}
