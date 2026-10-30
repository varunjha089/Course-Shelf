//! Moving a course folder to another drive: copy, verify, and only then delete the originals.
//! Nothing is removed before every file has been checked at the destination.

use anyhow::{bail, Context, Result};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Clone)]
pub struct MovePlan {
    pub src: PathBuf,
    pub dst: PathBuf,
    /// (file name, bytes)
    pub files: Vec<(String, u64)>,
    pub total_bytes: u64,
    /// Same filesystem: a move is an instant rename and needs no extra space.
    pub same_fs: bool,
    /// Files already at the destination with identical size (skipped, not copied again).
    pub already_there: usize,
}

/// Which disk holds `p`: two paths with the same id can be moved with a plain rename.
#[cfg(unix)]
fn dev(p: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().map(|m| m.dev())
}

/// Windows has no device number in the metadata: the drive letter (or network share) of the full path names the disk.
#[cfg(not(unix))]
fn dev(p: &Path) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let full = p.canonicalize().ok()?;
    let std::path::Component::Prefix(prefix) = full.components().next()? else { return None };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    prefix.as_os_str().to_string_lossy().to_uppercase().hash(&mut h);
    Some(h.finish())
}

impl MovePlan {
    /// Bytes that a copy would really write: files already at the destination with the same size are skipped.
    pub fn bytes_to_copy(&self) -> u64 {
        self.files.iter().filter(|(n, sz)| std::fs::metadata(self.dst.join(n)).map(|m| m.len() != *sz).unwrap_or(true)).map(|(_, sz)| *sz).sum()
    }
}

/// What would move. Hidden files (the `.courseshelf` state folder, `.DS_Store`) and sub-folders stay behind.
pub fn plan(src: &Path, dst: &Path) -> Result<MovePlan> {
    if !src.is_dir() {
        bail!("{} is not a folder", src.display());
    }
    if src.canonicalize().ok() == dst.canonicalize().ok() && dst.exists() {
        bail!("the source and destination are the same folder");
    }
    let mut files = Vec::new();
    let mut already = 0;
    for e in std::fs::read_dir(src)?.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let md = e.metadata()?;
        if name.starts_with('.') || !md.is_file() {
            continue;
        }
        if let Ok(dm) = std::fs::metadata(dst.join(&name)) {
            if dm.len() == md.len() {
                already += 1;
            } else {
                bail!("{name} already exists at the destination with a different size; resolve it first");
            }
        }
        files.push((name, md.len()));
    }
    files.sort();
    let total_bytes: u64 = files.iter().map(|f| f.1).sum();
    let probe = if dst.exists() { dst.to_path_buf() } else { dst.ancestors().find(|p| p.exists()).unwrap_or(Path::new("/")).to_path_buf() };
    Ok(MovePlan { src: src.to_path_buf(), dst: dst.to_path_buf(), files, total_bytes, same_fs: dev(src).is_some() && dev(src) == dev(&probe), already_there: already })
}

/// FNV over the first, middle and last MiB plus the size: cheap, and catches truncation and bit rot in practice.
pub fn sample_hash(p: &Path) -> Result<u64> {
    let mut f = std::fs::File::open(p)?;
    let len = f.metadata()?.len();
    let mut h: u64 = 0xcbf29ce484222325 ^ len;
    let mut buf = vec![0u8; 1 << 20];
    for off in [0, len / 2, len.saturating_sub(1 << 20)] {
        f.seek(SeekFrom::Start(off))?;
        let n = f.read(&mut buf)?;
        for b in &buf[..n] {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    Ok(h)
}

fn copy_file(from: &Path, to: &Path, cancel: &AtomicBool, on_bytes: &mut dyn FnMut(u64)) -> Result<()> {
    let tmp = to.with_file_name(format!(".part-{}", to.file_name().unwrap().to_string_lossy()));
    let mut r = std::fs::File::open(from)?;
    let mut w = std::fs::File::create(&tmp)?;
    let mut buf = vec![0u8; 8 << 20];
    loop {
        if cancel.load(Ordering::SeqCst) {
            drop(w);
            let _ = std::fs::remove_file(&tmp);
            bail!("cancelled");
        }
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n])?;
        on_bytes(n as u64);
    }
    w.sync_all()?;
    drop(w);
    std::fs::rename(&tmp, to)?;
    Ok(())
}

/// Copy (or rename, on the same filesystem), verify every file, then delete the originals unless `keep_original`.
/// `progress(bytes_done, total_bytes, current_file)`.
pub fn run(plan: &MovePlan, keep_original: bool, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, u64, &str)) -> Result<()> {
    std::fs::create_dir_all(&plan.dst).with_context(|| format!("cannot create {}", plan.dst.display()))?;
    let mut done = 0u64;
    let rename_only = plan.same_fs && !keep_original;
    for (name, size) in &plan.files {
        let (from, to) = (plan.src.join(name), plan.dst.join(name));
        progress(done, plan.total_bytes, name);
        if rename_only {
            if !to.exists() {
                std::fs::rename(&from, &to)?;
            } else if std::fs::metadata(&to)?.len() == *size && sample_hash(&to)? == sample_hash(&from)? {
                // the identical file is already there: the original is redundant
                std::fs::remove_file(&from)?;
            } else {
                bail!("{name} exists at the destination but differs from the original; nothing was overwritten");
            }
            done += size;
            continue;
        }
        if to.exists() && std::fs::metadata(&to)?.len() == *size && sample_hash(&to)? == sample_hash(&from)? {
            done += size;
            continue;
        }
        let base = done;
        copy_file(&from, &to, cancel, &mut |n| {
            done += n;
            progress(done, plan.total_bytes, name);
        })?;
        done = base + size;
    }
    if rename_only {
        let _ = std::fs::remove_dir(&plan.src);
        return Ok(());
    }
    // verify everything at the destination BEFORE any original is removed
    for (name, size) in &plan.files {
        let (from, to) = (plan.src.join(name), plan.dst.join(name));
        let m = std::fs::metadata(&to).with_context(|| format!("{name} is missing at the destination"))?;
        if m.len() != *size {
            bail!("{name}: destination has {} bytes, expected {size}; originals were NOT deleted", m.len());
        }
        if sample_hash(&to)? != sample_hash(&from)? {
            bail!("{name}: destination differs from the original; originals were NOT deleted");
        }
    }
    if !keep_original {
        for (name, _) in &plan.files {
            std::fs::remove_file(plan.src.join(name))?;
        }
        let _ = std::fs::remove_dir(&plan.src); // only succeeds when nothing else is left
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cs_mv_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn fill(d: &Path) {
        std::fs::write(d.join("001 - a.mp4"), vec![7u8; 3_000_000]).unwrap();
        std::fs::write(d.join("002 - b.mp4"), vec![9u8; 10]).unwrap();
        std::fs::write(d.join(".hidden"), b"keep me").unwrap();
    }

    #[test]
    fn copy_verify_delete() {
        let (s, d) = (dir("s1"), dir("d1").join("out"));
        fill(&s);
        let p = plan(&s, &d).unwrap();
        assert_eq!(p.files.len(), 2);
        assert_eq!(p.total_bytes, 3_000_010);
        let mut last = 0;
        // force the copy path by keeping originals, then check they are still there
        run(&p, true, &AtomicBool::new(false), &mut |done, _, _| last = done).unwrap();
        assert_eq!(last, 3_000_010);
        assert!(s.join("001 - a.mp4").is_file() && d.join("001 - a.mp4").is_file());
        // now a real move: destination already holds identical files, originals are removed afterwards
        run(&p, false, &AtomicBool::new(false), &mut |_, _, _| {}).unwrap();
        assert!(!s.join("001 - a.mp4").exists());
        assert!(s.join(".hidden").is_file(), "hidden files stay");
        assert!(d.join("002 - b.mp4").is_file());
    }

    #[test]
    fn corrupted_copy_is_recopied_and_never_trusted() {
        let (s, d) = (dir("s2"), dir("d2").join("out"));
        fill(&s);
        std::fs::create_dir_all(&d).unwrap();
        // a truncated stray file at the destination with a different size is refused up front
        std::fs::write(d.join("001 - a.mp4"), b"short").unwrap();
        assert!(plan(&s, &d).is_err());
        std::fs::remove_file(d.join("001 - a.mp4")).unwrap();
        let p = plan(&s, &d).unwrap();
        run(&p, true, &AtomicBool::new(false), &mut |_, _, _| {}).unwrap();
        // flip one byte in the middle of the copy (same size): a copy run must detect it and re-copy
        let mut data = std::fs::read(d.join("001 - a.mp4")).unwrap();
        data[1_500_000] ^= 0xff;
        std::fs::write(d.join("001 - a.mp4"), data).unwrap();
        run(&p, true, &AtomicBool::new(false), &mut |_, _, _| {}).unwrap();
        assert_eq!(std::fs::read(d.join("001 - a.mp4")).unwrap()[1_500_000], 7);
        // on one filesystem, a differing destination file is an error and nothing is lost
        let mut data = std::fs::read(d.join("001 - a.mp4")).unwrap();
        data[1_500_000] ^= 0xff;
        std::fs::write(d.join("001 - a.mp4"), data).unwrap();
        assert!(run(&p, false, &AtomicBool::new(false), &mut |_, _, _| {}).is_err());
        assert!(s.join("001 - a.mp4").is_file());
    }

    #[test]
    fn bytes_to_copy_skips_what_is_already_there() {
        let (s, d) = (dir("s4"), dir("d4").join("out"));
        fill(&s);
        let p = plan(&s, &d).unwrap();
        assert_eq!(p.bytes_to_copy(), 3_000_010);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("002 - b.mp4"), vec![9u8; 10]).unwrap();
        assert_eq!(p.bytes_to_copy(), 3_000_000);
    }

    #[test]
    fn same_folder_refused_and_cancel() {
        let s = dir("s3");
        fill(&s);
        assert!(plan(&s, &s).is_err());
        let d = dir("d3").join("o");
        let p = plan(&s, &d).unwrap();
        assert!(run(&p, true, &AtomicBool::new(true), &mut |_, _, _| {}).is_err());
        assert!(s.join("001 - a.mp4").is_file());
        assert!(!d.join("001 - a.mp4").exists());
    }
}
