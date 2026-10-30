# Windows build issues

Tried on Windows 11 (10.0.26300), Rust 1.99.1 (`x86_64-pc-windows-gnu`), Node.js 26.13.0, commit cloned from `main` on 2026-10-30.

The README says the core also builds on Windows. It does not with the current code.

## 1. `courseshelf-core` fails to compile (blocking)

Command: `cargo build --release -p courseshelf-fetch`

| # | Error | Location | Cause |
|---|-------|----------|-------|
| 1 | `E0433: cannot find unix in os` | `crates/core/src/mover.rs:6` | `use std::os::unix::fs::MetadataExt;` is Unix-only |
| 2 | `E0599: no method named dev found for struct Metadata` | `crates/core/src/mover.rs:24` | `m.dev()` (device id, used to tell whether two paths are on the same disk) comes from `MetadataExt` |
| 3 | `E0425: cannot find type statvfs in crate libc` | `crates/core/src/config.rs:321` | `libc::statvfs` does not exist on Windows |
| 4 | `E0425: cannot find function statvfs in crate libc` | `crates/core/src/config.rs:322` | same (free-space lookup) |

### Suggested fix

Gate the Unix code with `#[cfg(unix)]` and add `#[cfg(windows)]` fallbacks:

- Free space (`config.rs`): `GetDiskFreeSpaceExW` (via the `windows-sys` crate or `fs2`/`fs4`).
- Same-disk check (`mover.rs`): compare volume serial numbers (`GetFileInformationByHandle`), or compare drive-letter prefixes as a simpler approximation.

## 2. Missing tools on this machine

Not checked in as blockers, but needed before the app is usable:

- `yt-dlp`, `ffmpeg` (with `ffprobe`) and `deno` are not installed (needed for downloads).
- `pdfunite` and `pdfinfo` (poppler) are not installed (needed only to join papers into books).
- The README gives only `brew` commands. Windows equivalents (for example `winget install yt-dlp.yt-dlp Gyan.FFmpeg DenoLand.Deno`) are not documented.

## 3. Not yet attempted

- The Tauri app build (`cd app && npm install && npx tauri build`). It needs the core to compile first. It also needs the MSVC toolchain and the WebView2 runtime, and the README only describes producing `CourseShelf.app` for macOS. Windows bundling is undocumented.
- `app/src-tauri` and `crates/server` may contain more Unix-only code. It is hidden until `courseshelf-core` compiles.

## 4. Minor

- `npm run build` in `web/` succeeds but prints a chunk-size warning (some chunks over 500 kB). It is harmless.
- `CLAUDE.md` says to edit files on macOS with Python or the Edit tool because of BSD `sed`. That does not apply on Windows.
- The Windows data folder is `%APPDATA%\CourseShelf\`, per the README. This is untested because nothing runs yet.
