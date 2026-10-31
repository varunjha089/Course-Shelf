//! locations.json: where a course lives when that is NOT `<drive>/<course>`.
//! Lets you point a course at a folder you already downloaded, without touching courses.csv.

use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::{Config, Drive};
use crate::spec::CourseRow;

/// Where a course's folder is, and where its watch progress is kept.
#[derive(Debug, Clone)]
pub struct Located {
    pub dir: PathBuf,
    pub state_dir: PathBuf,
    /// The folder comes from locations.json, not from `<drive>/<course>`.
    pub overridden: bool,
}

/// Where a course lives: an explicit location (locations.json) wins, else `<drive root>/<course>` of the drive courses.csv
/// names. The app and `courseshelf-fetch` both use this, so they always agree.
pub fn locate(cfg: &Config, home: &Path, row: &CourseRow) -> Result<Located> {
    if let Some(p) = load(home).get(&row.course) {
        return Ok(Located { dir: p.clone(), state_dir: p.join(".courseshelf"), overridden: true });
    }
    let d = cfg.drive(&row.drive)?;
    Ok(Located { dir: d.root.join(&row.course), state_dir: d.state_dir(), overridden: false })
}

/// The drive a course's folder is really on, for free space and budget: the drive courses.csv names, or for a course with
/// an explicit location the configured drive that folder lies inside, else the folder itself (no budget).
pub fn host_drive(cfg: &Config, row: &CourseRow, loc: &Located) -> Result<Drive> {
    if !loc.overridden {
        return Ok(cfg.drive(&row.drive)?.clone());
    }
    Ok(cfg
        .drives
        .iter()
        .find(|d| d.connected() && relative_inside(&loc.dir, &d.root).is_some())
        .cloned()
        .unwrap_or_else(|| Drive { name: "folder".into(), root: loc.dir.clone(), budget_gb: None }))
}

fn file(home: &Path) -> PathBuf {
    home.join("locations.json")
}

pub fn load(home: &Path) -> BTreeMap<String, PathBuf> {
    std::fs::read_to_string(file(home))
        .ok()
        .and_then(|t| serde_json::from_str::<BTreeMap<String, String>>(&t).ok())
        .map(|m| m.into_iter().map(|(k, v)| (k, PathBuf::from(v))).collect())
        .unwrap_or_default()
}

fn save(home: &Path, m: &BTreeMap<String, PathBuf>) -> Result<()> {
    let plain: BTreeMap<&String, String> = m.iter().map(|(k, v)| (k, v.to_string_lossy().to_string())).collect();
    let tmp = file(home).with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&plain)?)?;
    std::fs::rename(&tmp, file(home))?;
    Ok(())
}

pub fn set(home: &Path, course: &str, path: &Path) -> Result<()> {
    crate::spec::check_course_name(course)?;
    if !path.is_absolute() {
        bail!("the folder must be an absolute path");
    }
    if !path.is_dir() {
        bail!("{} is not a folder", path.display());
    }
    let mut m = load(home);
    m.insert(course.to_string(), path.to_path_buf());
    save(home, &m)
}

pub fn clear(home: &Path, course: &str) -> Result<()> {
    let mut m = load(home);
    if m.remove(course).is_some() {
        save(home, &m)?;
    }
    Ok(())
}

/// `path` relative to `root` ("Provider/Course" for `<root>/Provider/Course`), when it lies inside it; links resolved,
/// `/` between parts. A drive scan also finds a course whose location is inside that drive: this names that folder,
/// so it is listed once, under the course's own name.
pub fn relative_inside(path: &Path, root: &Path) -> Option<String> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let rel = path.strip_prefix(&root).ok()?;
    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cs_loc_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn set_load_clear() {
        let home = tmp("a");
        let folder = tmp("b");
        assert!(load(&home).is_empty());
        set(&home, "MIT/LA", &folder).unwrap();
        assert_eq!(load(&home).get("MIT/LA"), Some(&folder));
        assert!(set(&home, "MIT/LA", Path::new("relative")).is_err());
        assert!(set(&home, "../x", &folder).is_err());
        assert!(set(&home, "MIT/LA", &home.join("nope")).is_err());
        clear(&home, "MIT/LA").unwrap();
        assert!(load(&home).is_empty());
        let _ = std::fs::remove_dir_all(&home);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn locate_follows_locations_json() {
        let home = tmp("loc_home");
        let (mac, other) = (tmp("loc_mac"), tmp("loc_other"));
        let cfg_text = format!(
            "courses_csv = \"courses.csv\"\n[drives.local]\npath = {:?}\n[drives.ssd]\npath = {:?}\nbudget_gb = 10\n",
            mac.to_string_lossy(),
            home.join("unplugged").to_string_lossy()
        );
        std::fs::write(home.join("config.toml"), cfg_text).unwrap();
        let cfg = Config::load(&home).unwrap();
        let row = CourseRow {
            course: "P/C".into(),
            playlist_url: String::new(),
            items: crate::items::ItemSpec::parse("all").unwrap(),
            items_text: "all".into(),
            priority: 1,
            drive: "ssd".into(),
            quality_cap: None,
            enabled: true,
            note: String::new(),
            line: 2,
        };

        let plain = locate(&cfg, &home, &row).unwrap();
        assert!(!plain.overridden);
        assert_eq!(host_drive(&cfg, &row, &plain).unwrap().name, "ssd");

        let on_mac = mac.join("Somewhere").join("C");
        std::fs::create_dir_all(&on_mac).unwrap();
        set(&home, "P/C", &on_mac).unwrap();
        let l = locate(&cfg, &home, &row).unwrap();
        assert!(l.overridden && l.dir == on_mac && l.state_dir == on_mac.join(".courseshelf"));
        assert_eq!(host_drive(&cfg, &row, &l).unwrap().name, "local", "a folder inside the Mac drive is on the Mac");

        set(&home, "P/C", &other).unwrap();
        let l = locate(&cfg, &home, &row).unwrap();
        let h = host_drive(&cfg, &row, &l).unwrap();
        assert_eq!((h.name.as_str(), h.budget_gb), ("folder", None));
        for d in [home, mac, other] {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn folder_inside_a_drive() {
        let root = tmp("root");
        let course = root.join("Extra").join("Provider").join("Course");
        std::fs::create_dir_all(&course).unwrap();
        assert_eq!(relative_inside(&course, &root).as_deref(), Some("Extra/Provider/Course"));
        assert_eq!(relative_inside(&root, &root), None, "the drive itself is not a course folder");
        let elsewhere = tmp("elsewhere");
        assert_eq!(relative_inside(&elsewhere, &root), None);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&elsewhere);
    }
}
