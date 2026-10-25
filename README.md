# CourseShelf

An offline library for video courses and PDFs, for macOS (Rust + Tauri; the core also builds on Linux and Windows). Download the lectures you
want from YouTube playlists once, then watch them offline with resume and ticks, keep them on several disks, read and annotate PDFs and books,
and browse a paper library.

| | `courseshelf-fetch` (download) | `CourseShelf.app` (watch and read) |
|---|---|---|
| Used | online, occasionally, drive plugged in | offline, daily |
| Form | Rust command-line tool around `yt-dlp` | Tauri 2 app (Rust server + React UI) |
| Job | download the lectures listed in `courses.csv`, verify them | browse courses, play, resume, tick lectures done, read PDFs |

Nothing in the app touches the network unless you press a download button; nothing in the downloader touches your watch progress.

## Getting started

1. **Tools**: [Rust](https://rustup.rs), [Node.js](https://nodejs.org) 18+, and for downloading `yt-dlp`, `ffmpeg` (with `ffprobe`) and `deno`
   (`brew install yt-dlp ffmpeg deno` on a Mac). `pdfunite` and `pdfinfo` (`brew install poppler`) are needed only to join papers into books.
2. **Build**:
   ```sh
   git clone https://github.com/varunjha089/Course-Shelf.git && cd Course-Shelf
   cd web && npm install && npm run build && cd ..      # the UI (the server embeds web/dist)
   cd app && npm install && npx tauri build && cd ..    # CourseShelf.app in target/release/bundle/macos/
   cargo build --release -p courseshelf-fetch           # the downloader
   ```
   Copy `CourseShelf.app` to /Applications (keep only one copy, or Launchpad shows two).
3. **First start** makes your **data folder** with a starter `config.toml` and an empty `courses.csv`. Nothing of yours is ever kept in the program
   folder, so you can update or move the program freely.
4. Edit `config.toml` (your drives: the folders that hold courses), add courses to `courses.csv`, then `courseshelf-fetch plan` and `courseshelf-fetch run`,
   or use the Downloads tab in the app.

## Where your data is

| | Default |
|---|---|
| macOS | `~/Library/Application Support/CourseShelf/` |
| Linux | `~/.config/courseshelf/` |
| Windows | `%APPDATA%\CourseShelf\` |

To keep your data somewhere else (a synced folder, an existing CourseShelf folder): `courseshelf-fetch home --set /path/to/folder`. That writes the folder
into `home.txt` in the settings folder above; nothing is copied or moved. `courseshelf-fetch home` shows which folder is in use. For a single run,
`COURSESHELF_HOME=/path` overrides both. The app shows the folder on its Downloads tab.

In the data folder: `config.toml`, `courses.csv` (you edit these), and what the app keeps: `annotations/` (PDF marks, never written into the PDF),
`collections/` + `collections.json` (books joined from papers), `bookmarks.json`, `singles.json`, `locations.json`, `ui-settings.json`, `papers/inbox/`
(downloaded papers), `cache/`. Back this folder up; it is everything that is yours. On each drive: `<drive>/<course>/NNN - Title.mp4` (+ `.info.json`,
`.en.vtt`) and a hidden `<drive>/.courseshelf/` with `progress.sqlite` (positions and ticks travel with the disk).

## courses.csv

```csv
course,playlist_url,items,priority,drive,quality_cap,enabled,note
MIT/18.06_Linear_Algebra,https://www.youtube.com/playlist?list=…,"1-10,12",1,local,,yes,first course
```

| Column | Meaning |
|---|---|
| `course` | Folder on the drive. One slash level allowed: `MIT/Linear_Algebra_18_06`. |
| `playlist_url` | YouTube playlist. |
| `items` | Playlist positions: `all`, `1-5,8`, or `all-2,15` (everything except). |
| `priority` | Lower = downloaded first, and kept first when a drive's budget runs out. |
| `drive` | A drive name from `config.toml` (for example `local`, `hdd`). |
| `quality_cap` | Blank = best available. A number such as `720` caps the height. |
| `enabled` | `yes` / `no`. `no` keeps the row but skips it in `run`. |
| `note` | Free text. |

Lines starting with `#` are comments. Changing the CSV and running again downloads only what is missing. Removing items never deletes anything by itself.
`courseshelf-fetch import-plans PLAN.md…` builds `courses.csv` from markdown plan files: a `## Provider Course title` heading per course, a
`Playlist: <url>` line, and a table whose 2nd column is the playlist position, 3rd the title and 5th the mark: `KEEP` rows become `items`, `KEEP (specialised)` only with
`--include-specialised`, any other mark is left out; all rows start with `enabled = no`. See `examples/course_plan.md`.
Examples of both files are in `examples/`.

## The downloader

```sh
F=./target/release/courseshelf-fetch
$F doctor                            # tools, drives, courses.csv all OK?
$F list "<playlist url>"             # positions + titles of a playlist (to write `items`)
$F plan                              # DRY RUN: what would download, size, free space
$F run                               # download what is missing (resumes, skips finished lectures)
$F run --only MIT/18.06_Linear_Algebra
$F prune                             # lectures no longer in `items`: lists them, deletes only if you type yes
$F home [--set FOLDER]               # show or choose the data folder
```

## The app's tabs

- **Courses**: every folder with videos on your drives (up to 3 levels deep), with resume, ticks and "Copy to Mac" for studying without the disk.
- **Downloads**: download lectures, point a course at a folder you downloaded by hand, rename hand-downloaded files to the playlist titles, move a course
  to another drive (ticks and positions come along), and **single videos**: paste a YouTube link, download it to any folder. "Remove link" never deletes a file.
- **Papers**: a view of a paper list CSV, in-app reading, question banks (`banks_dir`), arXiv / direct-PDF download into `papers/inbox/` (never into the library),
  and "Make a book" from chosen papers.
- **Add papers** (Papers tab): paste arXiv ids or links, or DOIs; they are looked up on arXiv / Crossref, papers already in the list are marked (matched on arXiv id,
  DOI or link) and cannot be added twice; choose a topic and a note for each, then Add. This is the only time CourseShelf writes the paper list. If your library has
  its own rules (ordering, numbering, checks), give its tool in `[papers] add_command = ["python3", "/path/to/add_papers.py"]`: CourseShelf writes the chosen papers
  to a JSON file and runs the command with that file as its last argument, in the library folder (exit code 0 = added; its output is the job's log). Without
  `add_command` the rows are appended to the end of the list after a backup copy in `<data folder>/papers/backups/`.
- **Which paper list**: with no setup, CourseShelf keeps its own list in `<data folder>/papers/papers.csv`, made by your first **Add papers**. To use a library of
  your own instead, add `[papers]` to `config.toml` with `library = "/path/to/folder"` (the list defaults to `Paper_Database.csv` in it; set `list = "..."` for
  another name). The list needs at least the columns `Title` and `Link`; CourseShelf also reads `S. No`, `DL_Topic`, `Local Link` (a `file://` link to the PDF)
  and `Notes`, for example:
  ```csv
  S. No,Title,DL_Topic,Link,Local Link,Notes
  1,Attention Is All You Need,Transformers,https://arxiv.org/abs/1706.03762,,
  ```
- **Books**: every PDF under the folders in `[pdf_sources]`, with pen, highlighter, shapes, text and pictures; marks are stored apart from the PDF and follow the
  file's content, so a moved or renamed PDF keeps its marks. A ☆ bookmarks a book.

## Development

`cargo test` (core and server unit tests), `cd web && npx tsc --noEmit`. Run the viewer in a browser: `./target/debug/courseshelf-serve --port 8765 --ui web/dist`
and open the printed URL (it carries a per-run key; the server listens on 127.0.0.1 only). To test without your own data, give it a scratch folder:
`COURSESHELF_HOME=/tmp/cs-test courseshelf-serve` (a starter config is written there). How the code is organised: `CLAUDE.md`.

Licence: MIT (see `LICENSE`).
