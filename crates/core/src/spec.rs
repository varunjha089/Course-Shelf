//! courses.csv: one row per course. This file is the only thing you edit.

use crate::items::ItemSpec;
use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct CourseRow {
    /// Folder under the drive root. May contain one slash level, e.g. `MIT/Linear_Algebra_18_06`.
    pub course: String,
    pub playlist_url: String,
    pub items: ItemSpec,
    /// The items text exactly as written in the CSV.
    pub items_text: String,
    /// Lower number = downloaded first and kept first when a drive budget runs out.
    pub priority: u32,
    pub drive: String,
    /// Highest vertical resolution wanted, e.g. 720. None = best available.
    pub quality_cap: Option<u32>,
    pub enabled: bool,
    pub note: String,
    /// Optional `order` column = `reverse`: the playlist lists the lectures newest first, so lecture N is the N-th video from
    /// the END (files are numbered that way, and `items` counts that way). New uploads then keep the old numbers.
    pub reverse: bool,
    /// 1-based line number in the file, for error messages.
    pub line: usize,
}

pub fn parse_bool(s: &str) -> Result<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "yes" | "y" | "true" | "1" | "on" => Ok(true),
        "no" | "n" | "false" | "0" | "off" | "" => Ok(false),
        other => bail!("'{other}' is not yes/no"),
    }
}

/// A course name becomes a folder path on disk, so it must stay inside the drive root.
pub fn check_course_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("course name is empty");
    }
    if name.starts_with('/') || name.contains("..") || name.contains('\\') || name.contains(':') {
        bail!("course name '{name}' must be a plain relative folder name");
    }
    for seg in name.split('/') {
        if seg.is_empty() || seg.starts_with('.') {
            bail!("course name '{name}' has an empty or hidden folder segment");
        }
    }
    if name.split('/').count() > 3 {
        bail!("course name '{name}' is nested more than 3 levels");
    }
    Ok(())
}

pub fn load_courses(path: &Path) -> Result<Vec<CourseRow>> {
    let mut rdr = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .flexible(true)
        .comment(Some(b'#'))
        .from_path(path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    let headers = rdr.headers()?.clone();
    let col = |name: &str| headers.iter().position(|h| h.eq_ignore_ascii_case(name));
    let need = |name: &str| col(name).ok_or_else(|| anyhow!("courses.csv has no '{name}' column"));
    let (c_course, c_url, c_items, c_drive) = (need("course")?, need("playlist_url")?, need("items")?, need("drive")?);
    let (c_prio, c_cap, c_en, c_note, c_order) = (col("priority"), col("quality_cap"), col("enabled"), col("note"), col("order"));

    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    for rec in rdr.records() {
        let rec = rec?;
        let line = rec.position().map(|p| p.line() as usize).unwrap_or(0);
        let get = |i: usize| rec.get(i).unwrap_or("").to_string();
        let opt = |i: Option<usize>| i.map(|i| get(i)).unwrap_or_default();
        let ctx = |what: &str| format!("courses.csv line {line}: {what}");

        let course = get(c_course);
        check_course_name(&course).with_context(|| ctx("course"))?;
        if !seen.insert(course.to_ascii_lowercase()) {
            bail!("{}", ctx(&format!("course '{course}' appears twice")));
        }
        let playlist_url = get(c_url);
        if !playlist_url.starts_with("https://") || !playlist_url.contains("list=") {
            // A row with no usable playlist is allowed only if disabled.
            let enabled = parse_bool(&opt(c_en)).unwrap_or(false);
            if enabled {
                bail!("{}", ctx(&format!("'{course}' is enabled but playlist_url is not a https://…list=… link")));
            }
        }
        let items = ItemSpec::parse(&get(c_items)).with_context(|| ctx("items"))?;
        let priority = match opt(c_prio).as_str() {
            "" => 100,
            s => s.parse::<u32>().with_context(|| ctx("priority must be a whole number"))?,
        };
        let quality_cap = match opt(c_cap).as_str() {
            "" | "best" | "0" => None,
            s => Some(s.trim_end_matches('p').parse::<u32>().with_context(|| ctx("quality_cap must be a number like 720"))?),
        };
        let enabled = parse_bool(&opt(c_en)).with_context(|| ctx("enabled"))?;
        let reverse = match opt(c_order).to_ascii_lowercase().as_str() {
            "" | "playlist" => false,
            "reverse" | "reversed" => true,
            other => bail!("{}", ctx(&format!("order '{other}' must be empty, playlist or reverse"))),
        };
        rows.push(CourseRow {
            course,
            playlist_url,
            items,
            items_text: get(c_items),
            priority,
            drive: get(c_drive),
            quality_cap,
            enabled,
            note: opt(c_note),
            reverse,
            line,
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct Tmp(std::path::PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn write(contents: &str) -> Tmp {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!("cs_test_{}_{}.csv", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        std::fs::File::create(&p).unwrap().write_all(contents.as_bytes()).unwrap();
        Tmp(p)
    }

    const HEAD: &str = "course,playlist_url,items,priority,drive,quality_cap,enabled,note\n";

    #[test]
    fn loads_a_row() {
        let t = write(&format!("{HEAD}MIT/LA,https://www.youtube.com/playlist?list=PLx,\"1-3,5\",2,hdd,720,yes,hello\n"));
        let rows = load_courses(&t.0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].course, "MIT/LA");
        assert_eq!(rows[0].quality_cap, Some(720));
        assert_eq!(rows[0].items.resolve(9), vec![1, 2, 3, 5]);
        assert!(rows[0].enabled);
    }

    #[test]
    fn optional_columns_may_be_missing() {
        let t = write("course,playlist_url,items,drive\nA,https://youtube.com/playlist?list=P,all,hdd\n");
        let rows = load_courses(&t.0).unwrap();
        assert_eq!(rows[0].priority, 100);
        assert!(!rows[0].enabled);
    }

    #[test]
    fn order_column() {
        let t = write("course,playlist_url,items,drive,note,order\nA,https://y/?list=1,all,hdd,,reverse\nB,https://y/?list=2,all,hdd,\"x, y\"\n");
        let rows = load_courses(&t.0).unwrap();
        assert!(rows[0].reverse && !rows[1].reverse, "a row without the column keeps playlist order");
        let t = write("course,playlist_url,items,drive,order\nA,https://y/?list=1,all,hdd,backwards\n");
        assert!(load_courses(&t.0).is_err());
    }

    #[test]
    fn rejects_traversal_and_duplicates() {
        let t = write(&format!("{HEAD}../x,https://y/?list=1,all,1,hdd,,no,\n"));
        assert!(load_courses(&t.0).is_err());
        let t = write(&format!("{HEAD}A,https://y/?list=1,all,1,hdd,,no,\nA,https://y/?list=2,all,1,hdd,,no,x\n"));
        assert!(load_courses(&t.0).is_err());
    }

    #[test]
    fn enabled_needs_a_url() {
        let t = write(&format!("{HEAD}A,,all,1,hdd,,yes,\n"));
        assert!(load_courses(&t.0).is_err());
        let t = write(&format!("{HEAD}A,,all,1,hdd,,no,need url\n"));
        assert!(load_courses(&t.0).is_ok());
    }
}
