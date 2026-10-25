//! config.toml: where the drives are and how yt-dlp should behave.
//!
//! ```toml
//! courses_csv = "courses.csv"          # relative to this file
//! [drives.hdd]
//! path = "/Volumes/CourseHDD/CourseShelf"
//! budget_gb = 800
//! ```

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct DriveCfg {
    pub path: String,
    /// Most this program may fill on the drive. Leave out for "whatever is free".
    pub budget_gb: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct YtdlpCfg {
    pub sleep_min: u32,
    pub sleep_max: u32,
    pub subtitle_langs: String,
    /// Estimated size of one hour of video at the chosen quality, only used by `plan` before real sizes are known.
    pub est_gb_per_hour: f64,
    pub extra_args: Vec<String>,
}

impl Default for YtdlpCfg {
    fn default() -> Self {
        YtdlpCfg { sleep_min: 3, sleep_max: 10, subtitle_langs: "en,-live_chat".into(), est_gb_per_hour: 1.3, extra_args: vec![] }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PapersCfg {
    pub library: String,
    /// The paper list inside the library (default Paper_Database.csv).
    pub list: Option<String>,
    /// The folder of question-bank PDFs inside the library (default Question_Full).
    pub banks_dir: Option<String>,
    pub inbox: Option<String>,
    pub banks: Option<String>,
    /// The library's own tool for adding papers (program and arguments); it gets a JSON file of new papers as its last argument.
    /// Without it, "Add papers" appends rows to the end of the list (after a backup).
    #[serde(default)]
    pub add_command: Vec<String>,
}

/// Extra folders of PDFs (books, a downloaded paper pile) the Books tab lists and the PDF viewer can annotate.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PdfSourcesCfg {
    #[serde(default)]
    pub folders: Vec<String>,
}

/// Which drive is "this Mac", and how much of it courses may use (for the "Copy to Mac" button).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct MacCfg {
    pub drive: Option<String>,
    pub max_gb: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawConfig {
    mac: Option<MacCfg>,
    pdf_sources: Option<PdfSourcesCfg>,
    papers: Option<PapersCfg>,
    courses_csv: Option<String>,
    #[serde(default)]
    drives: BTreeMap<String, DriveCfg>,
    #[serde(default)]
    ytdlp: YtdlpCfg,
}

#[derive(Debug, Clone)]
pub struct Drive {
    pub name: String,
    pub root: PathBuf,
    pub budget_gb: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct PapersPaths {
    /// The paper library folder (holds the paper list and the PDFs its Local Link column points to).
    pub library: PathBuf,
    /// The paper list CSV (`[papers] list`, default <library>/Paper_Database.csv).
    pub list_csv: PathBuf,
    /// Question-bank PDFs (`[papers] banks_dir`, default <library>/Question_Full).
    pub banks_dir: PathBuf,
    pub inbox: PathBuf,
    pub banks_csv: PathBuf,
    /// `[papers] add_command` (empty: append to the list).
    pub add_command: Vec<String>,
    /// No `[papers]` section: the list is CourseShelf's own starter list in the data folder, which "Add papers" may create.
    pub starter: bool,
}

impl PapersPaths {
    /// The paper setup when config.toml has no `[papers]` section: everything in `<data folder>/papers/`, the list `papers.csv`
    /// (made by the first "Add papers"). A configured library is never created this way.
    pub fn starter(home: &Path) -> PapersPaths {
        let library = home.join("papers");
        PapersPaths {
            list_csv: library.join("papers.csv"),
            banks_dir: library.join("Question_Full"),
            inbox: library.join("inbox"),
            banks_csv: library.join("banks.csv"),
            library,
            add_command: vec![],
            starter: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub mac: MacCfg,
    pub pdf_folders: Vec<PathBuf>,
    pub papers: Option<PapersPaths>,
    pub home: PathBuf,
    pub courses_csv: PathBuf,
    pub drives: Vec<Drive>,
    pub ytdlp: YtdlpCfg,
}

pub fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(h) = std::env::var_os("HOME") {
            return PathBuf::from(h).join(rest);
        }
    }
    PathBuf::from(p)
}

/// The folder for CourseShelf's own settings on this computer (not the user's data): macOS ~/Library/Application Support/CourseShelf,
/// Windows %APPDATA%\\CourseShelf, Linux $XDG_CONFIG_HOME/courseshelf (or ~/.config/courseshelf).
pub fn settings_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        return expand_home("~/Library/Application Support/CourseShelf");
    }
    if cfg!(windows) {
        if let Some(a) = std::env::var_os("APPDATA") {
            return PathBuf::from(a).join("CourseShelf");
        }
    }
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(x) if !x.is_empty() => PathBuf::from(x).join("courseshelf"),
        _ => expand_home("~/.config/courseshelf"),
    }
}

/// The file that says where the data folder is, when it is not the settings folder itself.
pub fn home_pointer() -> PathBuf {
    settings_dir().join("home.txt")
}

/// Where config.toml, courses.csv and the user's data (annotations, books, bookmarks, ...) live:
/// 1. $COURSESHELF_HOME, 2. the folder written in <settings folder>/home.txt, 3. the settings folder itself.
/// The program folder never holds user data, so it can be updated, moved or shared freely.
pub fn default_home() -> PathBuf {
    if let Some(h) = std::env::var_os("COURSESHELF_HOME") {
        return PathBuf::from(h);
    }
    if let Ok(t) = std::fs::read_to_string(home_pointer()) {
        let t = t.trim();
        if !t.is_empty() {
            return expand_home(t);
        }
    }
    settings_dir()
}

/// A first config.toml for a new data folder: one drive in the user's home folder, nothing else. Written only when there is none.
pub const STARTER_CONFIG: &str = r#"# CourseShelf configuration (made on first start; edit it freely, the app re-reads it).
# Each drive is a folder that holds course folders (and a hidden .courseshelf/ folder with your watch progress).

courses_csv = "courses.csv"

# The drive that is this computer, and how much course video may be kept on it in total.
[mac]
drive = "local"
max_gb = 50

[drives.local]
path = "~/CourseShelf Courses"

# An external disk (remove the # to use it). budget_gb keeps CourseShelf from filling it.
# [drives.hdd]
# path = "/Volumes/MyDisk/Courses"
# budget_gb = 200

[ytdlp]
sleep_min = 3                    # seconds to wait between videos, so YouTube does not throttle you
sleep_max = 10
subtitle_langs = "en,-live_chat"
est_gb_per_hour = 1.3            # only used by `plan` for its size estimate
extra_args = []                  # e.g. ["--cookies-from-browser", "chrome"]

# Optional: a paper library for the Papers tab (a folder with a paper list CSV). CourseShelf writes it only through "Add papers".
# Without this section the Papers tab keeps its own list in papers/papers.csv in this data folder (made by the first "Add papers").
# [papers]
# library = "~/Papers"
# list = "Paper_Database.csv"     # columns: S. No, Title, Link, Local Link, ... (see README)
# inbox = "papers/inbox"          # downloaded papers go here, never into the library
# add_command = ["python3", "/path/to/add_papers.py"]   # optional: the library's own tool for "Add papers" (else rows are appended)

# Optional: folders of PDFs for the Books tab (to 5 levels deep). Annotations are kept in this data folder, never inside the PDF.
# [pdf_sources]
# folders = ["~/Documents/Books"]
"#;

pub const STARTER_COURSES: &str = "course,playlist_url,items,priority,drive,quality_cap,enabled,note\n";

/// Make a new data folder usable: create it, and write a starter config.toml and an empty courses.csv when they are missing.
/// Existing files are never changed. Returns the files it wrote.
pub fn ensure_home(home: &Path) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(home).with_context(|| format!("cannot create {}", home.display()))?;
    let mut made = vec![];
    for (name, text) in [("config.toml", STARTER_CONFIG), ("courses.csv", STARTER_COURSES)] {
        let p = home.join(name);
        if !p.exists() {
            std::fs::write(&p, text).with_context(|| format!("cannot write {}", p.display()))?;
            made.push(p);
        }
    }
    Ok(made)
}

impl Config {
    pub fn load(home: &Path) -> Result<Config> {
        let path = home.join("config.toml");
        let text = std::fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
        let raw: RawConfig = toml::from_str(&text).with_context(|| format!("{} is not valid TOML", path.display()))?;
        if raw.drives.is_empty() {
            bail!("config.toml defines no [drives.*]");
        }
        let courses_csv = {
            let p = expand_home(raw.courses_csv.as_deref().unwrap_or("courses.csv"));
            if p.is_absolute() { p } else { home.join(p) }
        };
        let drives = raw
            .drives
            .into_iter()
            .map(|(name, d)| Drive { name, root: expand_home(&d.path), budget_gb: d.budget_gb })
            .collect();
        let papers = raw.papers.map(|p| {
            let rel = |s: &str| {
                let x = expand_home(s);
                if x.is_absolute() { x } else { home.join(x) }
            };
            let library = rel(&p.library);
            let inside = |s: Option<&str>, default: &str| {
                let x = expand_home(s.unwrap_or(default));
                if x.is_absolute() { x } else { library.join(x) }
            };
            PapersPaths {
                list_csv: inside(p.list.as_deref(), "Paper_Database.csv"),
                banks_dir: inside(p.banks_dir.as_deref(), "Question_Full"),
                library,
                inbox: rel(p.inbox.as_deref().unwrap_or("papers/inbox")),
                banks_csv: rel(p.banks.as_deref().unwrap_or("papers/banks.csv")),
                add_command: p.add_command.clone(),
                starter: false,
            }
        });
        let pdf_folders = raw.pdf_sources.unwrap_or_default().folders.iter().map(|f| { let x = expand_home(f); if x.is_absolute() { x } else { home.join(x) } }).collect();
        Ok(Config { mac: raw.mac.unwrap_or_default(), pdf_folders, papers, home: home.to_path_buf(), courses_csv, drives, ytdlp: raw.ytdlp })
    }

    /// The drive that is this Mac: `[mac] drive`, else the first drive whose folder is not under /Volumes.
    pub fn mac_drive(&self) -> Option<&Drive> {
        if let Some(n) = &self.mac.drive {
            return self.drives.iter().find(|d| &d.name == n);
        }
        self.drives.iter().find(|d| !d.root.starts_with("/Volumes"))
    }

    pub fn mac_max_bytes(&self) -> Option<u64> {
        self.mac.max_gb.map(|g| (g * 1e9) as u64)
    }

    pub fn drive(&self, name: &str) -> Result<&Drive> {
        self.drives.iter().find(|d| d.name == name).ok_or_else(|| {
            anyhow!("drive '{name}' is not in config.toml (known: {})", self.drives.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", "))
        })
    }
}

impl Drive {
    /// The mount point to test for "is the disk plugged in": /Volumes/<X> for external disks, else the path itself or its parent.
    pub fn mount_point(&self) -> PathBuf {
        let mut comps = self.root.components();
        let s = self.root.to_string_lossy();
        if let Some(rest) = s.strip_prefix("/Volumes/") {
            let first = rest.split('/').next().unwrap_or("");
            return PathBuf::from("/Volumes").join(first);
        }
        let _ = comps.next();
        if self.root.exists() { self.root.clone() } else { self.root.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| self.root.clone()) }
    }

    pub fn connected(&self) -> bool {
        self.mount_point().exists()
    }

    /// (free_bytes, total_bytes) of the filesystem holding the drive root.
    pub fn space(&self) -> Option<(u64, u64)> {
        let mut p = self.root.clone();
        while !p.exists() {
            if !p.pop() {
                return None;
            }
        }
        let c = std::ffi::CString::new(p.to_string_lossy().as_bytes()).ok()?;
        unsafe {
            let mut st: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(c.as_ptr(), &mut st) != 0 {
                return None;
            }
            let bs = st.f_frsize as u64;
            Some((st.f_bavail as u64 * bs, st.f_blocks as u64 * bs))
        }
    }

    /// Create the library root and its hidden state folder (with a drive id) if missing.
    pub fn init(&self) -> Result<()> {
        if !self.connected() {
            bail!("drive '{}' is not connected ({})", self.name, self.mount_point().display());
        }
        let meta = self.root.join(".courseshelf");
        std::fs::create_dir_all(&meta)?;
        let idf = meta.join("drive.json");
        if !idf.exists() {
            let id = crate::random_hex(8)?;
            std::fs::write(&idf, format!("{{\"id\":\"{id}\",\"name\":\"{}\"}}\n", self.name))?;
        }
        Ok(())
    }

    pub fn state_dir(&self) -> PathBuf {
        self.root.join(".courseshelf")
    }
}

/// Change `path = "..."` of one `[drives.NAME]` block in config.toml, keeping every comment and every other line.
pub fn set_drive_path(home: &Path, name: &str, new_path: &str) -> Result<()> {
    if new_path.contains('"') || new_path.contains('\n') || !new_path.starts_with('/') {
        bail!("the drive folder must be an absolute path without quotes");
    }
    let file = home.join("config.toml");
    let text = std::fs::read_to_string(&file).with_context(|| format!("cannot read {}", file.display()))?;
    let header = format!("[drives.{name}]");
    let mut out: Vec<String> = Vec::new();
    let (mut in_block, mut done) = (false, false);
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_block = t == header;
        } else if in_block && !done && t.starts_with("path") && t.contains('=') {
            out.push(format!("path = \"{new_path}\""));
            done = true;
            continue;
        }
        out.push(line.to_string());
    }
    if !done {
        bail!("drive '{name}' has no path line in config.toml");
    }
    let mut result = out.join("\n");
    if text.ends_with('\n') {
        result.push('\n');
    }
    toml::from_str::<toml::Value>(&result).context("the edited config.toml would not be valid; nothing was changed")?;
    let tmp = file.with_extension("toml.tmp");
    std::fs::write(&tmp, result)?;
    std::fs::rename(&tmp, &file)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_start_makes_a_usable_home_and_never_overwrites() {
        let d = std::env::temp_dir().join(format!("cs_home_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let made = ensure_home(&d.join("new")).unwrap();
        assert_eq!(made.len(), 2);
        let c = Config::load(&d.join("new")).unwrap();                         // the starter config is valid
        assert_eq!(c.drives.len(), 1);
        assert!(c.drives[0].root.ends_with("CourseShelf Courses"));
        assert!(crate::spec::load_courses(&c.courses_csv).unwrap().is_empty());
        std::fs::write(d.join("new").join("config.toml"), "[drives.x]\npath = \"/tmp/x\"\n").unwrap();
        assert!(ensure_home(&d.join("new")).unwrap().is_empty());              // existing files stay as they are
        assert!(std::fs::read_to_string(d.join("new").join("config.toml")).unwrap().contains("/tmp/x"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn paper_library_names_have_defaults_and_can_be_changed() {
        let d = std::env::temp_dir().join(format!("cs_pap_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("config.toml"), "[drives.x]\npath = \"/tmp/x\"\n[papers]\nlibrary = \"/lib\"\n").unwrap();
        let p = Config::load(&d).unwrap().papers.unwrap();
        assert!(!p.starter);
        assert_eq!((p.list_csv, p.banks_dir), (PathBuf::from("/lib/Paper_Database.csv"), PathBuf::from("/lib/Question_Full")));
        std::fs::write(d.join("config.toml"), "[drives.x]\npath = \"/tmp/x\"\n[papers]\nlibrary = \"/lib\"\nlist = \"my_papers.csv\"\nbanks_dir = \"/q\"\n").unwrap();
        let p = Config::load(&d).unwrap().papers.unwrap();
        assert!(!p.starter);
        assert_eq!((p.list_csv, p.banks_dir), (PathBuf::from("/lib/my_papers.csv"), PathBuf::from("/q")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn mac_drive_is_configured_or_detected() {
        let mk = |toml: &str| {
            let d = std::env::temp_dir().join(format!("cs_mac_{}_{}", std::process::id(), toml.len()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("config.toml"), toml).unwrap();
            let c = Config::load(&d).unwrap();
            let _ = std::fs::remove_dir_all(&d);
            c
        };
        let c = mk("[drives.hdd]\npath = \"/Volumes/X/y\"\n[drives.local]\npath = \"/Users/z/c\"\n");
        assert_eq!(c.mac_drive().unwrap().name, "local");
        assert_eq!(c.mac_max_bytes(), None);
        let c = mk("[mac]\ndrive = \"hdd\"\nmax_gb = 20\n[drives.hdd]\npath = \"/Volumes/X/y\"\n[drives.local]\npath = \"/Users/z/c\"\n");
        assert_eq!(c.mac_drive().unwrap().name, "hdd");
        assert_eq!(c.mac_max_bytes(), Some(20_000_000_000));
    }

    #[test]
    fn edits_only_the_named_drive_and_keeps_comments() {
        let d = std::env::temp_dir().join(format!("cs_cfg_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("config.toml"), "# top comment\n[drives.a]\npath = \"/old/a\"   # keep\nbudget_gb = 5\n\n[drives.b]\n# why\npath = \"/old/b\"\n").unwrap();
        set_drive_path(&d, "b", "/Volumes/New Disk/YouTube").unwrap();
        let t = std::fs::read_to_string(d.join("config.toml")).unwrap();
        assert!(t.contains("path = \"/Volumes/New Disk/YouTube\""));
        assert!(t.contains("path = \"/old/a\"   # keep"), "other drive untouched");
        assert!(t.contains("# top comment") && t.contains("# why") && t.contains("budget_gb = 5"));
        assert!(set_drive_path(&d, "zzz", "/x").is_err());
        assert!(set_drive_path(&d, "a", "relative").is_err());
        assert!(set_drive_path(&d, "a", "/has\"quote").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn external_mount_point() {
        let d = Drive { name: "x".into(), root: PathBuf::from("/Volumes/Foo/CourseShelf/sub"), budget_gb: None };
        assert_eq!(d.mount_point(), PathBuf::from("/Volumes/Foo"));
        assert!(!d.connected());
    }

    #[test]
    fn tilde() {
        if std::env::var_os("HOME").is_some() {
            assert!(expand_home("~/a").is_absolute());
            assert!(expand_home("~/a").ends_with("a"));
        }
        assert_eq!(expand_home("/x/y"), PathBuf::from("/x/y"));
    }
}
