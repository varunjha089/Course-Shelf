//! Finding the command-line tools (yt-dlp, ffmpeg, ffprobe, deno, pdfinfo, ...).
//!
//! A Mac app opened from Finder does NOT get your terminal's PATH: it gets launchd's bare
//! `/usr/bin:/bin:/usr/sbin:/sbin`, which has no Homebrew. So every tool is looked up in the
//! inherited PATH first and then in the places package managers install to, and the child process
//! is given that same extended PATH (yt-dlp itself needs ffmpeg and deno to be findable).
//! On Windows the same is done with winget's, Scoop's and Chocolatey's folders, and `name.exe` is found for `name`.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(not(windows))]
fn extra_dirs() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/opt/homebrew/sbin"), PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/local/sbin")];
    if let Some(h) = crate::config::home_dir() {
        v.push(h.join(".local/bin"));
        v.push(h.join(".cargo/bin"));
        v.push(h.join(".deno/bin"));
    }
    v.extend(["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(PathBuf::from));
    v
}

#[cfg(windows)]
fn extra_dirs() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let mut v = Vec::new();
    if let Some(l) = env("LOCALAPPDATA") {
        v.push(l.join(r"Microsoft\WinGet\Links"));
    }
    if let Some(h) = crate::config::home_dir() {
        v.push(h.join(r"scoop\shims"));
        v.push(h.join(r".cargo\bin"));
        v.push(h.join(r".deno\bin"));
    }
    if let Some(p) = env("ProgramData") {
        v.push(p.join(r"chocolatey\bin"));
    }
    v
}

/// The directories searched, in order, without duplicates: the inherited PATH, then the usual install places.
pub fn search_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("PATH") {
        out.extend(std::env::split_paths(&p));
    }
    out.extend(extra_dirs());
    let mut seen = std::collections::HashSet::new();
    out.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.clone()));
    out
}

pub fn extended_path() -> OsString {
    std::env::join_paths(search_dirs()).unwrap_or_default()
}

/// Absolute path of `name`, or None if it is not installed anywhere we look.
pub fn find(name: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) && Path::new(name).extension().is_none() { vec![format!("{name}.exe"), name.to_string()] } else { vec![name.to_string()] };
    if name.contains('/') || name.contains(std::path::MAIN_SEPARATOR) {
        return names.into_iter().map(PathBuf::from).find(|p| p.is_file());
    }
    search_dirs().into_iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

/// A Command for `name` that works whether or not the app inherited a useful PATH.
pub fn command(name: &str) -> Command {
    let mut c = Command::new(find(name).unwrap_or_else(|| PathBuf::from(name)));
    c.env("PATH", extended_path());
    c
}

/// Open a file, folder or web address with the system's default program (`open` on a Mac, Explorer on Windows, `xdg-open`
/// elsewhere); with `reveal`, show the file selected in its folder instead.
pub fn open(target: &OsStr, reveal: bool) -> std::io::Result<()> {
    let status = if cfg!(target_os = "macos") {
        let mut c = command("open");
        if reveal {
            c.arg("-R");
        }
        c.arg(target).status()?
    } else if cfg!(windows) {
        let mut c = Command::new("explorer");
        if reveal {
            select_arg(&mut c, target);
        } else {
            c.arg(target);
        }
        // explorer.exe exits with 1 even when it worked: only a failure to start it is an error
        c.status()?;
        return Ok(());
    } else {
        let p = Path::new(target);
        let t = if reveal && p.is_file() { p.parent().map(|d| d.as_os_str()).unwrap_or(target) } else { target };
        command("xdg-open").arg(t).status()?
    };
    if status.success() { Ok(()) } else { Err(std::io::Error::other(format!("the open command exited with {status}"))) }
}

/// Explorer wants `/select,"C:\a b\file"` as one argument, with its own quotes, which Rust's quoting would not produce.
#[cfg(windows)]
fn select_arg(c: &mut Command, target: &OsStr) {
    use std::os::windows::process::CommandExt;
    c.raw_arg(format!("/select,\"{}\"", target.to_string_lossy()));
}

#[cfg(not(windows))]
fn select_arg(c: &mut Command, target: &OsStr) {
    c.arg(target);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn finds_system_tools_and_is_not_fooled_by_a_bare_path() {
        assert!(find("ls").is_some());
        assert!(find("definitely-not-a-real-tool-xyz").is_none());
        // the extended PATH always contains the Homebrew and system locations, whatever was inherited
        let p = extended_path().to_string_lossy().to_string();
        assert!(p.contains("/opt/homebrew/bin") && p.contains("/usr/bin"));
    }

    #[cfg(unix)]
    #[test]
    fn child_sees_the_extended_path() {
        let out = command("sh").arg("-c").arg("echo $PATH").output().unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("/opt/homebrew/bin"));
    }

    #[cfg(windows)]
    #[test]
    fn finds_windows_tools_without_the_exe_suffix() {
        assert!(find("cmd").is_some_and(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))));
        assert!(find("definitely-not-a-real-tool-xyz").is_none());
        let out = command("cmd").args(["/C", "echo %PATH%"]).output().unwrap();
        assert!(out.status.success());
    }
}
