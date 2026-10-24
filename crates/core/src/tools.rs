//! Finding the command-line tools (yt-dlp, ffmpeg, ffprobe, deno, pdfinfo, ...).
//!
//! A Mac app opened from Finder does NOT get your terminal's PATH: it gets launchd's bare
//! `/usr/bin:/bin:/usr/sbin:/sbin`, which has no Homebrew. So every tool is looked up in the
//! inherited PATH first and then in the places package managers install to, and the child process
//! is given that same extended PATH (yt-dlp itself needs ffmpeg and deno to be findable).

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

fn extra_dirs() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/opt/homebrew/sbin"), PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/local/sbin")];
    if let Some(h) = std::env::var_os("HOME") {
        let h = PathBuf::from(h);
        v.push(h.join(".local/bin"));
        v.push(h.join(".cargo/bin"));
        v.push(h.join(".deno/bin"));
    }
    v.extend(["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(PathBuf::from));
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
    if name.contains('/') {
        let p = PathBuf::from(name);
        return p.is_file().then_some(p);
    }
    search_dirs().into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// A Command for `name` that works whether or not the app inherited a useful PATH.
pub fn command(name: &str) -> Command {
    let mut c = Command::new(find(name).unwrap_or_else(|| PathBuf::from(name)));
    c.env("PATH", extended_path());
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_system_tools_and_is_not_fooled_by_a_bare_path() {
        assert!(find("ls").is_some());
        assert!(find("definitely-not-a-real-tool-xyz").is_none());
        // the extended PATH always contains the Homebrew and system locations, whatever was inherited
        let p = extended_path().to_string_lossy().to_string();
        assert!(p.contains("/opt/homebrew/bin") && p.contains("/usr/bin"));
    }

    #[test]
    fn child_sees_the_extended_path() {
        let out = command("sh").arg("-c").arg("echo $PATH").output().unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("/opt/homebrew/bin"));
    }
}
