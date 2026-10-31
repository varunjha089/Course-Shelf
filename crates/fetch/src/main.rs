//! courseshelf-fetch: download the lectures listed in courses.csv with yt-dlp.
//!
//!   courseshelf-fetch doctor              check yt-dlp / ffmpeg / drives / courses.csv
//!   courseshelf-fetch list <playlist-url> show a playlist's positions and titles (to write `items`)
//!   courseshelf-fetch plan                dry run: what would download, sizes, free space
//!   courseshelf-fetch run [--only NAME]   download what is missing
//!   courseshelf-fetch prune               list (and, if you confirm, delete) files no longer in `items`
//!   courseshelf-fetch import-plans FILE…  build courses.csv from the Video_Course_Plan*.md tables
//!   courseshelf-fetch tidy [FOLDER…]       shrink and hide the .info.json / .vtt side files (all course folders if none given)

mod importer;
use courseshelf_core::fetcher as ytdlp;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use courseshelf_core::{
    config::{default_home, Config, Drive},
    items::to_ytdlp,
    library,
    locations::{host_drive, locate},
    spec::{load_courses, CourseRow},
};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Parser)]
#[command(name = "courseshelf-fetch", about = "Download the courses listed in courses.csv (uses yt-dlp)")]
struct Cli {
    /// Data folder holding config.toml and courses.csv (default: $COURSESHELF_HOME, else the folder in <settings>/home.txt, else the settings folder)
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Check the tools, the drives and courses.csv.
    Doctor,
    /// Show the data folder, or keep your data in another folder: `home --set FOLDER` (written to home.txt in the settings folder).
    Home {
        #[arg(long)]
        set: Option<PathBuf>,
    },
    /// Show the videos of a playlist: position, length, title.
    List { url: String },
    /// Dry run: show what `run` would download. Downloads nothing.
    Plan {
        #[arg(long)]
        only: Option<String>,
    },
    /// Download every wanted lecture that is not on the drive yet.
    Run {
        #[arg(long)]
        only: Option<String>,
        /// Download even if the playlist titles do not match the plan's titles.
        #[arg(long)]
        force_titles: bool,
    },
    /// List files whose position is no longer in `items`; delete them only after you type yes.
    Prune {
        #[arg(long)]
        only: Option<String>,
    },
    /// Shrink each .info.json to what CourseShelf reads and hide the side files, so a course folder shows only its videos.
    /// Without folders: every course folder on the connected drives and in locations.json. Videos are never touched.
    Tidy { folders: Vec<PathBuf> },
    /// Build courses.csv from plan markdown files (all rows start disabled).
    ImportPlans {
        files: Vec<PathBuf>,
        /// Also put KEEP (specialised) lectures into `items`.
        #[arg(long)]
        include_specialised: bool,
        /// Overwrite an existing courses.csv.
        #[arg(long)]
        force: bool,
    },
}

fn main() {
    if let Err(e) = real_main() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    let cli = Cli::parse();
    if let Cmd::Home { set } = &cli.cmd {
        return home_cmd(set.as_deref());
    }
    let home = cli.home.unwrap_or_else(default_home);
    for f in courseshelf_core::config::ensure_home(&home)? {
        println!("first start: wrote {}", f.display());
    }
    match cli.cmd {
        Cmd::Doctor => doctor(&home),
        Cmd::List { url } => list(&home, &url),
        Cmd::Plan { only } => plan(&home, only.as_deref()),
        Cmd::Run { only, force_titles } => run(&home, only.as_deref(), force_titles),
        Cmd::Prune { only } => prune(&home, only.as_deref()),
        Cmd::Tidy { folders } => tidy(&home, folders),
        Cmd::ImportPlans { files, include_specialised, force } => importer::import(&home, &files, include_specialised, force),
        Cmd::Home { .. } => unreachable!(),
    }
}

// ---------- tidy ----------

fn tidy(home: &std::path::Path, folders: Vec<PathBuf>) -> Result<()> {
    let folders = if folders.is_empty() { courseshelf_core::tidy::course_folders(&Config::load(home)?, home, None) } else { folders };
    let (mut shrunk, mut saved, mut hidden) = (0, 0u64, 0);
    for dir in &folders {
        let r = courseshelf_core::tidy::tidy(dir).with_context(|| format!("{}", dir.display()))?;
        if r.shrunk > 0 || r.hidden > 0 {
            println!("{}: {} shrunk ({:.1} MB saved), {} hidden", dir.display(), r.shrunk, r.bytes_saved as f64 / 1e6, r.hidden);
        }
        (shrunk, saved, hidden) = (shrunk + r.shrunk, saved + r.bytes_saved, hidden + r.hidden);
    }
    println!("{} folder(s): {shrunk} details file(s) shrunk, {:.1} MB saved, {hidden} side file(s) hidden", folders.len(), saved as f64 / 1e6);
    Ok(())
}

// ---------- shared ----------

struct Ctx {
    cfg: Config,
    rows: Vec<CourseRow>,
}

fn load(home: &std::path::Path, only: Option<&str>) -> Result<Ctx> {
    let cfg = Config::load(home)?;
    let mut rows = load_courses(&cfg.courses_csv)?;
    for r in &rows {
        cfg.drive(&r.drive).with_context(|| format!("courses.csv line {}", r.line))?;
    }
    if let Some(o) = only {
        rows.retain(|r| r.course == o);
        if rows.is_empty() {
            bail!("no row named '{o}' in courses.csv");
        }
    }
    rows.sort_by_key(|r| (r.priority, r.line));
    Ok(Ctx { cfg, rows })
}

fn gb(b: f64) -> String {
    format!("{:.1} GB", b / 1e9)
}

fn size_factor(cap: Option<u32>) -> f64 {
    // Rough: bytes scale with pixel count; relative to 1080p.
    match cap {
        None => 1.0,
        Some(c) if c >= 1080 => 1.0,
        Some(c) if c >= 720 => 0.6,
        Some(c) if c >= 480 => 0.35,
        Some(_) => 0.2,
    }
}

// ---------- doctor ----------

fn tool_version(name: &str, arg: &str) -> Option<String> {
    let out = courseshelf_core::tools::command(name).arg(arg).output().ok()?;
    let text = String::from_utf8_lossy(if out.stdout.is_empty() { &out.stderr } else { &out.stdout }).to_string();
    text.lines().next().map(|s| s.trim().to_string())
}

/// Show the data folder, or point CourseShelf at another one (nothing is copied or moved: the folder is used as it is).
fn home_cmd(set: Option<&std::path::Path>) -> Result<()> {
    use courseshelf_core::config;
    if let Some(dir) = set {
        let dir = if dir.is_absolute() { dir.to_path_buf() } else { std::env::current_dir()?.join(dir) };
        std::fs::create_dir_all(config::settings_dir())?;
        std::fs::write(config::home_pointer(), format!("{}\n", dir.display()))?;
        println!("CourseShelf now keeps its data in {}", dir.display());
        for f in config::ensure_home(&dir)? {
            println!("wrote {}", f.display());
        }
        return Ok(());
    }
    let from = if std::env::var_os("COURSESHELF_HOME").is_some() {
        "$COURSESHELF_HOME".to_string()
    } else if config::home_pointer().exists() {
        config::home_pointer().display().to_string()
    } else {
        "the default (the settings folder)".to_string()
    };
    println!("data folder: {}\n(from {from})", default_home().display());
    Ok(())
}

fn doctor(home: &std::path::Path) -> Result<()> {
    let mut bad = 0;
    let mut check = |label: &str, v: Option<String>, need: bool| match v {
        Some(s) => println!("  ok    {label}: {s}"),
        None if need => {
            println!("  FAIL  {label}: not found");
            bad += 1;
        }
        None => println!("  warn  {label}: not found"),
    };
    println!("tools");
    check("yt-dlp", tool_version("yt-dlp", "--version"), true);
    check("ffmpeg", tool_version("ffmpeg", "-version"), true);
    check("ffprobe", tool_version("ffprobe", "-version"), true);
    check("deno (JS runtime for YouTube)", tool_version("deno", "--version"), false);

    println!("config  ({})", home.display());
    let cfg = match Config::load(home) {
        Ok(c) => c,
        Err(e) => {
            println!("  FAIL  {e:#}");
            bail!("fix config.toml first");
        }
    };
    println!("  ok    config.toml, {} drive(s)", cfg.drives.len());
    for d in &cfg.drives {
        if d.connected() {
            let (free, total) = d.space().unwrap_or((0, 0));
            println!("  ok    drive {:<6} {}  {} free of {}", d.name, d.root.display(), gb(free as f64), gb(total as f64));
        } else {
            println!("  warn  drive {:<6} {}  NOT CONNECTED", d.name, d.root.display());
        }
    }
    match load_courses(&cfg.courses_csv) {
        Ok(rows) => {
            let on = rows.iter().filter(|r| r.enabled).count();
            println!("  ok    {} : {} row(s), {} enabled", cfg.courses_csv.display(), rows.len(), on);
            for r in &rows {
                if cfg.drive(&r.drive).is_err() {
                    println!("  FAIL  line {}: drive '{}' is not in config.toml", r.line, r.drive);
                    bad += 1;
                }
            }
        }
        Err(e) => {
            println!("  FAIL  {e:#}");
            bad += 1;
        }
    }
    if bad > 0 {
        bail!("{bad} problem(s) found");
    }
    println!("all good");
    Ok(())
}

// ---------- list ----------

fn list(home: &std::path::Path, url: &str) -> Result<()> {
    let pl = ytdlp::playlist(home, url, false)?;
    println!("{}  ({} videos)", pl.title, pl.entries.len());
    for e in &pl.entries {
        println!("{:>4}  {:>8}  {}", e.index, ytdlp::fmt_dur(e.duration), e.title);
    }
    Ok(())
}

// ---------- plan / run ----------

struct Todo<'a> {
    row: &'a CourseRow,
    /// The drive the course's folder is on (see `locations::host_drive`).
    drive: Drive,
    /// The course's folder (locations.json wins over `<drive>/<course>`).
    dir: PathBuf,
    total: u32,
    wanted: Vec<u32>,
    missing: Vec<u32>,
    est_bytes: f64,
    title_warnings: Vec<String>,
    note: String,
}

fn build_todos<'a>(ctx: &'a Ctx, home: &std::path::Path) -> Result<Vec<Todo<'a>>> {
    let mut todos = Vec::new();
    for row in ctx.rows.iter().filter(|r| r.enabled) {
        let loc = locate(&ctx.cfg, home, row)?;
        let drive = host_drive(&ctx.cfg, row, &loc)?;
        let dir = loc.dir;
        let pl = match ytdlp::course_playlist(home, row, false) {
            Ok(p) => p,
            Err(e) => {
                todos.push(Todo { row, drive, dir, total: 0, wanted: vec![], missing: vec![], est_bytes: 0.0, title_warnings: vec![], note: format!("cannot read playlist: {e:#}") });
                continue;
            }
        };
        let total = pl.entries.len() as u32;
        let wanted = row.items.resolve(total);
        let have: BTreeSet<u32> = if drive.connected() { library::downloaded_indexes(&dir) } else { BTreeSet::new() };
        let missing: Vec<u32> = wanted.iter().copied().filter(|i| !have.contains(i)).collect();
        let secs: f64 = missing.iter().filter_map(|i| pl.entries.get(*i as usize - 1)).map(|e| e.duration.unwrap_or(0.0)).sum();
        let est_bytes = secs / 3600.0 * ctx.cfg.ytdlp.est_gb_per_hour * 1e9 * size_factor(row.quality_cap);
        let mut note = String::new();
        if let Some(m) = row.items.max_named() {
            if m > total {
                note = format!("items mention {m} but the playlist has only {total} videos");
            }
        }
        let title_warnings = ytdlp::verify_titles(home, row, &pl);
        todos.push(Todo { row, drive, dir, total, wanted, missing, est_bytes, title_warnings, note });
    }
    Ok(todos)
}

/// Decide which todos fit on each drive, by priority. Returns for each todo: Some(reason) if it is skipped.
fn budget_cut(todos: &[Todo]) -> Vec<Option<String>> {
    let mut used: std::collections::HashMap<String, f64> = Default::default();
    let mut out = Vec::new();
    for t in todos {
        if !t.drive.connected() {
            out.push(Some(format!("drive '{}' not connected", t.drive.name)));
            continue;
        }
        let free = t.drive.space().map(|s| s.0 as f64).unwrap_or(f64::MAX);
        let cap = t.drive.budget_gb.map(|g| g * 1e9).unwrap_or(f64::MAX).min(free * 0.97);
        let u = used.entry(t.drive.name.clone()).or_insert(0.0);
        if *u + t.est_bytes > cap {
            out.push(Some(format!("over budget: needs ~{} but only ~{} left on '{}'", gb(t.est_bytes), gb((cap - *u).max(0.0)), t.drive.name)));
        } else {
            *u += t.est_bytes;
            out.push(None);
        }
    }
    out
}

fn plan(home: &std::path::Path, only: Option<&str>) -> Result<()> {
    let ctx = load(home, only)?;
    let todos = build_todos(&ctx, home)?;
    if todos.is_empty() {
        println!("No enabled rows in {}. Set enabled=yes on a row to include it.", ctx.cfg.courses_csv.display());
        return Ok(());
    }
    let cut = budget_cut(&todos);
    println!("{:<4} {:<34} {:<6} {:>6} {:>7} {:>8} {:>10}  status", "pri", "course", "drive", "wanted", "have", "missing", "est size");
    let mut total_missing = 0usize;
    let mut total_bytes = 0.0;
    for (t, c) in todos.iter().zip(&cut) {
        let have = t.wanted.len() - t.missing.len();
        let status = if !t.note.is_empty() && t.total == 0 {
            t.note.clone()
        } else if let Some(r) = c {
            r.clone()
        } else if !t.title_warnings.is_empty() {
            format!("TITLE MISMATCH ({}): check items", t.title_warnings.len())
        } else if t.missing.is_empty() {
            "up to date".into()
        } else {
            "will download".into()
        };
        println!("{:<4} {:<34} {:<6} {:>6} {:>7} {:>8} {:>10}  {}", t.row.priority, t.row.course, t.drive.name, t.wanted.len(), have, t.missing.len(), gb(t.est_bytes), status);
        if !t.note.is_empty() && t.total > 0 {
            println!("       note: {}", t.note);
        }
        for w in t.title_warnings.iter().take(5) {
            println!("       title check: {w}");
        }
        if c.is_none() {
            total_missing += t.missing.len();
            total_bytes += t.est_bytes;
        }
    }
    println!("\n{} lecture(s) to download, about {} (estimate at {} GB/hour; real sizes come from YouTube at download time).", total_missing, gb(total_bytes), ctx.cfg.ytdlp.est_gb_per_hour);
    for d in &ctx.cfg.drives {
        if let Some((free, _)) = d.space().filter(|_| d.connected()) {
            println!("drive {:<6} {} free{}", d.name, gb(free as f64), d.budget_gb.map(|b| format!(", budget {b} GB")).unwrap_or_default());
        }
    }
    println!("Nothing was downloaded.");
    Ok(())
}

fn run(home: &std::path::Path, only: Option<&str>, force_titles: bool) -> Result<()> {
    let ctx = load(home, only)?;
    ytdlp::require_tools()?;
    let todos = build_todos(&ctx, home)?;
    if todos.is_empty() {
        println!("No enabled rows in {}.", ctx.cfg.courses_csv.display());
        return Ok(());
    }
    let cut = budget_cut(&todos);
    let mut results: Vec<ytdlp::CourseResult> = Vec::new();
    for (t, c) in todos.iter().zip(&cut) {
        println!("\n=== {} ({}) ===", t.row.course, t.drive.name);
        if t.total == 0 {
            println!("skip: {}", t.note);
            continue;
        }
        if let Some(reason) = c {
            println!("skip: {reason}");
            continue;
        }
        if !t.title_warnings.is_empty() && !force_titles {
            println!("skip: the playlist's titles do not match the plan's titles for {} item(s):", t.title_warnings.len());
            for w in t.title_warnings.iter().take(8) {
                println!("   {w}");
            }
            println!("   fix `items`, or rerun with --force-titles if the playlist order really is right.");
            continue;
        }
        if t.missing.is_empty() {
            println!("up to date ({} lecture(s))", t.wanted.len());
            results.push(ytdlp::CourseResult::ok(t.row, t.wanted.len(), 0));
            continue;
        }
        t.drive.init()?;
        println!("downloading {} lecture(s): {}", t.missing.len(), to_ytdlp(&t.missing));
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let mut last_pct = -10.0f32;
        let mut sink = |ev: ytdlp::Event| match ev {
            ytdlp::Event::Line(l) => println!("{l}"),
            ytdlp::Event::Item { index, title } => println!("  -> {index:03} {title}"),
            ytdlp::Event::Progress { index, percent, speed, eta } => {
                if percent - last_pct >= 10.0 || percent >= 99.9 || percent < last_pct {
                    println!("     {index:03}  {percent:5.1}%  {speed}  eta {eta}");
                    last_pct = percent;
                }
            }
            ytdlp::Event::Checked { index, ok, message } => println!("  {} {index:03} {message}", if ok { "verified" } else { "FAILED  " }),
        };
        let r = ytdlp::download_course(&ctx.cfg, t.row, &t.missing, &t.dir, &mut sink, &cancel)?;
        println!("{}", r.summary());
        results.push(r);
    }
    ytdlp::write_status(&ctx.cfg, &results)?;
    let failed: usize = results.iter().map(|r| r.failed.len()).sum();
    if failed > 0 {
        bail!("{failed} lecture(s) failed verification or download; run again to retry them");
    }
    Ok(())
}

// ---------- prune ----------

fn prune(home: &std::path::Path, only: Option<&str>) -> Result<()> {
    let ctx = load(home, only)?;
    let mut doomed: Vec<PathBuf> = Vec::new();
    for row in &ctx.rows {
        let dir = locate(&ctx.cfg, home, row)?.dir;
        if !dir.is_dir() {
            continue;
        }
        let pl = match ytdlp::course_playlist(home, row, false) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let keep: BTreeSet<u32> = row.items.resolve(pl.entries.len() as u32).into_iter().collect();
        for l in library::scan_folder(&dir) {
            if l.index > 0 && !keep.contains(&l.index) {
                let stem = std::path::Path::new(&l.file).file_stem().unwrap().to_string_lossy().to_string();
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    for e in rd.flatten() {
                        if e.file_name().to_string_lossy().starts_with(&stem) {
                            doomed.push(e.path());
                        }
                    }
                }
            }
        }
    }
    if doomed.is_empty() {
        println!("Nothing to prune: every lecture on the drives is still in `items`.");
        return Ok(());
    }
    let bytes: u64 = doomed.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
    println!("These {} file(s), {}, are no longer wanted by courses.csv:", doomed.len(), gb(bytes as f64));
    for p in &doomed {
        println!("  {}", p.display());
    }
    print!("Delete them? Type yes to confirm: ");
    use std::io::Write;
    std::io::stdout().flush().ok();
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    if s.trim() == "yes" {
        for p in &doomed {
            std::fs::remove_file(p)?;
        }
        println!("Deleted.");
    } else {
        println!("Nothing deleted.");
    }
    Ok(())
}
