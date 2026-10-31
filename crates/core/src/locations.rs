//! locations.json: where a course lives when that is NOT `<drive>/<course>`.
//! Lets you point a course at a folder you already downloaded, without touching courses.csv.

use anyhow::{bail, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
