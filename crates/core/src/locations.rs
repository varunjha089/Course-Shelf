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
}
