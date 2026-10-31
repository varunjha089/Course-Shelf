//! Updating CourseShelf from its GitHub releases.
//!
//! `check` reads the latest published release (with curl, like the rest of the core) and picks the installer for this
//! system; `download` fetches it and checks its size and SHA-256; `install` puts it in place; `Restart::run` starts the new
//! version once this process has exited. Only the program is replaced: the data folder and the drives are never touched.
//!
//! | System  | Asset                                   | Install                                     |
//! |---------|-----------------------------------------|---------------------------------------------|
//! | macOS   | `CourseShelf_<v>_<arch>.app.zip`         | unzip next to the running app, then swap    |
//! | Windows | `*_x64-setup.exe` (NSIS)                 | run the installer after the app quits       |
//! | Linux   | `*_amd64.deb`                            | open it in the system's software installer  |

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The public repository the releases come from.
pub const REPO: &str = "varunjha089/Course-Shelf";

/// Overrides the "latest release" address (tests point it at a local server).
const URL_ENV: &str = "COURSESHELF_UPDATE_URL";

pub type Version = (u64, u64, u64);

#[derive(Debug, Clone, serde::Serialize)]
pub struct Release {
    pub version: String,
    pub tag: String,
    /// Release notes (Markdown).
    pub notes: String,
    /// The release's web page, for systems without a ready-made installer.
    pub page: String,
    pub asset: Option<Asset>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex SHA-256, when GitHub lists it.
    pub sha256: Option<String>,
}

/// What to start once the app has quit.
#[derive(Debug, Clone, PartialEq)]
pub enum Restart {
    /// macOS: open the updated `.app`.
    App(PathBuf),
    /// Windows: run the downloaded installer.
    Installer(PathBuf),
}

/// "v1.2.3", "1.2" or "1.2.3-beta" → (1, 2, 3).
pub fn parse_version(s: &str) -> Option<Version> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let mut parts = s.split(['-', '+']).next()?.split('.').map(|p| p.parse::<u64>().ok());
    let major = parts.next()??;
    let minor = parts.next().unwrap_or(Some(0))?;
    let patch = parts.next().unwrap_or(Some(0))?;
    Some((major, minor, patch))
}

fn latest_url() -> String {
    std::env::var(URL_ENV).unwrap_or_else(|_| format!("https://api.github.com/repos/{REPO}/releases/latest"))
}

/// The newest published release when it is newer than `current`; `Ok(None)` when this copy is up to date (or nothing has
/// been published yet).
pub fn check(current: &str) -> Result<Option<Release>> {
    let out = crate::tools::command("curl")
        .args(["-sSL", "--max-time", "20", "-H", "Accept: application/vnd.github+json", "-w", "\n%{http_code}"])
        .arg(latest_url())
        .output()
        .context("could not run curl")?;
    if !out.status.success() {
        bail!("GitHub could not be reached ({}). Check the internet connection and try again.", String::from_utf8_lossy(&out.stderr).trim());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text.rsplit_once('\n').unwrap_or((&text, ""));
    match code.trim() {
        "200" => {}
        "404" => return Ok(None),
        "403" | "429" => bail!("GitHub is busy right now. Please try again in an hour."),
        c => bail!("GitHub answered with HTTP {c}. Please try again later."),
    }
    let release = parse_release(body, std::env::consts::OS, std::env::consts::ARCH)?;
    let newer = parse_version(&release.version) > parse_version(current);
    Ok(newer.then_some(release))
}

/// Reads a GitHub "release" object and picks the asset for `os` / `arch`.
pub fn parse_release(json: &str, os: &str, arch: &str) -> Result<Release> {
    let v: serde_json::Value = serde_json::from_str(json).context("unexpected answer from GitHub")?;
    let tag = v["tag_name"].as_str().ok_or_else(|| anyhow!("the release has no version tag"))?.to_string();
    let version = parse_version(&tag).map(|(a, b, c)| format!("{a}.{b}.{c}")).ok_or_else(|| anyhow!("“{tag}” is not a version number"))?;
    let assets: Vec<Asset> = v["assets"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some(Asset {
                        name: a["name"].as_str()?.to_string(),
                        url: a["browser_download_url"].as_str()?.to_string(),
                        size: a["size"].as_u64().unwrap_or(0),
                        sha256: a["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(str::to_ascii_lowercase),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Release {
        version,
        notes: v["body"].as_str().unwrap_or_default().trim().to_string(),
        page: v["html_url"].as_str().map_or_else(|| format!("https://github.com/{REPO}/releases/latest"), str::to_string),
        asset: pick_asset(&assets, os, arch).cloned(),
        tag,
    })
}

/// The installer for this system.
pub fn pick_asset<'a>(assets: &'a [Asset], os: &str, arch: &str) -> Option<&'a Asset> {
    let find = |suffix: &str, archs: &[&str]| {
        assets.iter().find(|a| {
            let n = a.name.to_ascii_lowercase();
            n.ends_with(suffix) && archs.iter().any(|x| n.contains(x))
        })
    };
    match (os, arch) {
        ("macos", "aarch64") => find(".app.zip", &["aarch64", "arm64", "universal"]),
        ("macos", "x86_64") => find(".app.zip", &["x64", "x86_64", "universal"]),
        ("windows", "x86_64") => find("-setup.exe", &["x64", "x86_64"]),
        ("linux", "x86_64") => find(".deb", &["amd64", "x86_64"]),
        _ => None,
    }
}

/// Folder for downloads and unpacking.
pub fn work_dir() -> PathBuf {
    std::env::temp_dir().join("courseshelf-update")
}

/// Downloads `asset` into `dir` and checks its size and (when listed) SHA-256. A damaged file is removed.
pub fn download(asset: &Asset, dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    // the name comes from the server: keep only its last part
    let name = Path::new(&asset.name).file_name().ok_or_else(|| anyhow!("the download has no file name"))?;
    let dest = dir.join(name);
    let status = crate::tools::command("curl").args(["-fsSL", "--retry", "3", "-o"]).arg(&dest).arg(&asset.url).status().context("could not run curl")?;
    if !status.success() {
        let _ = std::fs::remove_file(&dest);
        bail!("the download failed; please try again later");
    }
    let size = std::fs::metadata(&dest)?.len();
    let fail = |msg: String| -> Result<PathBuf> {
        let _ = std::fs::remove_file(&dest);
        bail!(msg)
    };
    if asset.size > 0 && size != asset.size {
        return fail(format!("the download is incomplete ({size} of {} bytes)", asset.size));
    }
    if asset.sha256.as_ref().is_some_and(|want| sha256_of(&dest).ok().as_ref() != Some(want)) {
        return fail("the download is damaged (its SHA-256 does not match the release)".into());
    }
    Ok(dest)
}

fn sha256_of(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Puts the downloaded file in place. Returns what to start after quitting, or None when the system installer took over.
pub fn install(file: &Path) -> Result<Option<Restart>> {
    let name = file.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if name.ends_with(".app.zip") {
        let bundle = running_bundle().ok_or_else(|| anyhow!("this copy is not running from an .app, so it cannot replace itself; unzip {} by hand", file.display()))?;
        install_app_zip(file, &bundle)?;
        Ok(Some(Restart::App(bundle)))
    } else if name.ends_with(".exe") {
        Ok(Some(Restart::Installer(file.to_path_buf())))
    } else if name.ends_with(".deb") {
        crate::tools::open(file.as_os_str(), false).context("could not open the package")?;
        Ok(None)
    } else {
        bail!("do not know how to install {}", file.display())
    }
}

/// The `.app` this process runs from (`X.app/Contents/MacOS/exe`).
fn running_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let bundle = exe.parent()?.parent()?.parent()?;
    (bundle.extension()? == "app" && bundle.join("Contents/Info.plist").is_file()).then(|| bundle.to_path_buf())
}

/// Unzips the new `.app` next to `bundle`, then swaps the two; the old one is removed only once the new one is in place.
pub fn install_app_zip(zip: &Path, bundle: &Path) -> Result<()> {
    let parent = bundle.parent().ok_or_else(|| anyhow!("{} has no parent folder", bundle.display()))?;
    let stage = parent.join(format!(".courseshelf-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage)?;
    let result = (|| -> Result<()> {
        let out = Command::new("ditto").args(["-x", "-k"]).arg(zip).arg(&stage).output().context("could not run ditto")?;
        if !out.status.success() {
            bail!("could not unpack the update: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        let app = std::fs::read_dir(&stage)?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|x| x == "app") && p.join("Contents/Info.plist").is_file())
            .ok_or_else(|| anyhow!("the update contains no complete .app"))?;
        let old = parent.join(format!(".courseshelf-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&old);
        std::fs::rename(bundle, &old).with_context(|| format!("could not move the current app aside ({})", bundle.display()))?;
        if let Err(e) = std::fs::rename(&app, bundle) {
            let _ = std::fs::rename(&old, bundle);
            bail!("could not put the new app in place: {e}");
        }
        let _ = std::fs::remove_dir_all(&old);
        let _ = Command::new("xattr").args(["-dr", "com.apple.quarantine"]).arg(bundle).status();
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&stage);
    result
}

impl Restart {
    /// Starts the new version as soon as this process has exited.
    pub fn run(&self) -> std::io::Result<()> {
        match self {
            Restart::App(bundle) => Command::new("/bin/sh")
                .args(["-c", "while kill -0 \"$1\" 2>/dev/null; do sleep 0.2; done; exec open \"$2\"", "sh", &std::process::id().to_string()])
                .arg(bundle)
                .spawn()
                .map(|_| ()),
            Restart::Installer(path) => Command::new(path).spawn().map(|_| ()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        Asset { name: name.into(), url: format!("https://example.invalid/{name}"), size: 1, sha256: None }
    }

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert!(parse_version("v0.10.0") > parse_version("0.9.9"));
        assert_eq!(parse_version("latest"), None);
    }

    #[test]
    fn picks_the_installer_for_each_system() {
        let assets: Vec<Asset> =
            ["CourseShelf_0.2.0_aarch64.app.zip", "CourseShelf_0.2.0_x64-setup.exe", "course-shelf_0.2.0_amd64.deb", "courseshelf-fetch"].into_iter().map(asset).collect();
        let pick = |os, arch| pick_asset(&assets, os, arch).map(|a| a.name.as_str());
        assert_eq!(pick("macos", "aarch64"), Some("CourseShelf_0.2.0_aarch64.app.zip"));
        assert_eq!(pick("macos", "x86_64"), None, "no Intel build: the release page is offered");
        assert_eq!(pick("windows", "x86_64"), Some("CourseShelf_0.2.0_x64-setup.exe"));
        assert_eq!(pick("linux", "x86_64"), Some("course-shelf_0.2.0_amd64.deb"));
    }

    #[test]
    fn reads_a_release() {
        let json = r#"{"tag_name": "v0.3.0", "html_url": "https://github.com/o/r/releases/tag/v0.3.0", "body": " Notes ",
            "assets": [{"name": "CourseShelf_0.3.0_aarch64.app.zip", "browser_download_url": "https://x/a.zip", "size": 7, "digest": "sha256:ABC"}]}"#;
        let r = parse_release(json, "macos", "aarch64").unwrap();
        assert_eq!((r.version.as_str(), r.notes.as_str()), ("0.3.0", "Notes"));
        let a = r.asset.unwrap();
        assert_eq!((a.size, a.sha256.as_deref()), (7, Some("abc")));
        assert!(parse_release(json, "linux", "x86_64").unwrap().asset.is_none());
        assert!(parse_release("{}", "macos", "aarch64").is_err());
    }

    /// The whole Mac path on a scratch "app": zip a newer bundle, swap it in, and nothing else in the folder changes.
    #[cfg(target_os = "macos")]
    #[test]
    fn swaps_an_app_bundle_from_a_zip() {
        let base = std::env::temp_dir().join(format!("cs_upd_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let make = |root: &Path, version: &str| {
            let app = root.join("CourseShelf.app/Contents");
            std::fs::create_dir_all(app.join("MacOS")).unwrap();
            std::fs::write(app.join("Info.plist"), format!("<plist>{version}</plist>")).unwrap();
            std::fs::write(app.join("MacOS/courseshelf-app"), version).unwrap();
        };
        let (apps, new) = (base.join("Applications"), base.join("new"));
        make(&apps, "0.1.0");
        make(&new, "0.2.0");
        std::fs::write(apps.join("Other.txt"), "keep").unwrap();
        let zip = base.join("CourseShelf_0.2.0_aarch64.app.zip");
        assert!(Command::new("ditto").args(["-c", "-k", "--keepParent"]).arg(new.join("CourseShelf.app")).arg(&zip).status().unwrap().success());

        let bundle = apps.join("CourseShelf.app");
        install_app_zip(&zip, &bundle).unwrap();
        assert_eq!(std::fs::read_to_string(bundle.join("Contents/MacOS/courseshelf-app")).unwrap(), "0.2.0");
        let left: Vec<String> = std::fs::read_dir(&apps).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        let mut left = left;
        left.sort();
        assert_eq!(left, ["CourseShelf.app", "Other.txt"], "no staging or old copy left behind");
        let _ = std::fs::remove_dir_all(&base);
    }
}
