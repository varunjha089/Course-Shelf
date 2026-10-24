//! import-plans: build courses.csv from the Video_Course_Plan*.md tables, and check titles later.

use anyhow::{bail, Context, Result};
use courseshelf_core::{config::Config, items::to_csv_items};
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
struct Section {
    heading: String,
    playlist: String,
    /// (number, title, verdict)
    rows: Vec<(u32, String, String)>,
}

fn parse(md: &str) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for line in md.lines() {
        if let Some(h) = line.strip_prefix("## ").or_else(|| line.strip_prefix("### ")) {
            out.push(Section { heading: h.trim().to_string(), ..Default::default() });
        } else if let Some(u) = line.strip_prefix("Playlist:") {
            if let Some(s) = out.last_mut() {
                s.playlist = u.trim().to_string();
            }
        } else if line.starts_with('|') {
            let cells: Vec<&str> = line.trim().trim_matches('|').split('|').map(|c| c.trim()).collect();
            if cells.len() < 5 {
                continue;
            }
            let Ok(n) = cells[1].parse::<u32>() else { continue };
            if let Some(s) = out.last_mut() {
                s.rows.push((n, cells[2].to_string(), cells[4].to_string()));
            }
        }
    }
    out.retain(|s| !s.rows.is_empty());
    out
}

/// "MIT 18.100A Real Analysis (Fall 2020, …)" -> "MIT/18.100A_Real_Analysis": the first word of the heading is the provider (the folder the course goes
/// in); a provider of several words is written joined, "IIT_Roorkee Remote Sensing" -> "IIT_Roorkee/Remote_Sensing". A one-word heading goes under "Other".
fn course_name(heading: &str) -> String {
    let base = heading.split(" (").next().unwrap_or(heading).trim();
    let (group, rest) = match base.split_once(' ') {
        Some((g, r)) if !r.trim().is_empty() => (clean(g), r.trim()),
        _ => ("Other".to_string(), base),
    };
    format!("{group}/{}", clean(rest))
}

fn clean(s: &str) -> String {
    let c: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    c.split('_').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("_")
}

#[cfg(test)]
mod tests {
    use super::course_name;
    #[test]
    fn provider_is_the_first_word() {
        assert_eq!(course_name("MIT 18.100A Real Analysis (Fall 2020, Casey Rodriguez)"), "MIT/18.100A_Real_Analysis");
        assert_eq!(course_name("IIT_Roorkee Remote Sensing Essentials"), "IIT_Roorkee/Remote_Sensing_Essentials");
        assert_eq!(course_name("Calculus"), "Other/Calculus");
    }
}

pub fn import(home: &Path, files: &[PathBuf], include_specialised: bool, force: bool) -> Result<()> {
    if files.is_empty() {
        bail!("give at least one plan file, e.g. Video_Course_Plan.md");
    }
    let cfg = Config::load(home)?;
    let has_rows = std::fs::read_to_string(&cfg.courses_csv)
        .map(|t| t.lines().filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')).count() > 1)
        .unwrap_or(false);
    if has_rows && !force {
        bail!("{} already exists; add --force to overwrite it (your edits would be lost)", cfg.courses_csv.display());
    }
    let default_drive = cfg.drives.iter().find(|d| d.name == "hdd").or(cfg.drives.first()).map(|d| d.name.clone()).unwrap();
    std::fs::create_dir_all(home.join("plans"))?;
    let mut w = csv::Writer::from_path(&cfg.courses_csv)?;
    w.write_record(["course", "playlist_url", "items", "priority", "drive", "quality_cap", "enabled", "note"])?;
    let mut pri = 10;
    let mut n_rows = 0;
    for f in files {
        let md = std::fs::read_to_string(f).with_context(|| format!("cannot read {}", f.display()))?;
        for s in parse(&md) {
            let keep: Vec<u32> = s.rows.iter().filter(|r| r.2 == "KEEP").map(|r| r.0).collect();
            let spec: Vec<u32> = s.rows.iter().filter(|r| r.2.starts_with("KEEP (spec")).map(|r| r.0).collect();
            let opt: Vec<u32> = s.rows.iter().filter(|r| r.2 == "OPTIONAL").map(|r| r.0).collect();
            let mut chosen = keep.clone();
            if include_specialised {
                chosen.extend(&spec);
            }
            chosen.sort_unstable();
            chosen.dedup();
            let mut notes = vec![format!("from {}", f.file_name().unwrap().to_string_lossy())];
            if !include_specialised && !spec.is_empty() {
                notes.push(format!("specialised not included: {}", to_csv_items(&spec)));
            }
            if chosen.is_empty() {
                chosen = opt.clone();
                notes.push("only optional lectures".into());
            }
            if chosen.is_empty() {
                continue;
            }
            if s.playlist.is_empty() {
                notes.push("PLAYLIST URL NEEDED; check # matches playlist position".into());
            }
            let name = course_name(&s.heading);
            // titles for the safety check later
            let tsv: String = s.rows.iter().map(|(n, t, _)| format!("{n}\t{t}\n")).collect();
            std::fs::write(home.join("plans").join(format!("{}.tsv", name.replace('/', "__"))), tsv)?;
            w.write_record([&name, &s.playlist, &to_csv_items(&chosen), &pri.to_string(), &default_drive, "", "no", &notes.join("; ")])?;
            pri += 10;
            n_rows += 1;
        }
    }
    w.flush()?;
    println!("Wrote {} with {n_rows} course row(s), all enabled=no.", cfg.courses_csv.display());
    println!("Next: fill any blank playlist_url, set enabled=yes on what you want, then `courseshelf-fetch plan`.");
    Ok(())
}
