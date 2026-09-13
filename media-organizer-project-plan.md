# Media Library Organizer — Project Plan

A small, fast Windows desktop tool that scans a messy downloads folder, identifies films and TV series, organises them into clean folders with matched subtitles, and sets poster artwork as the folder icon.

---

## 1. Scope

### MVP (must have)
- Scan a chosen folder (recursively) for video files
- Parse filenames to guess title, year, season/episode
- Confirm identity against TMDB
- Classify as Movie or TV Series
- Move/rename into a clean library structure
- Match subtitle files to their video
- **Dry-run preview** before anything is touched
- **Undo log** so every operation is reversible

### v1 (should have)
- Download poster from TMDB → convert to `.ico` → set as folder icon
- Folder colour tinting
- Settings screen (library path, naming template, API key)
- Progress UI with per-file status

### v1.5 (nice to have)
- Auto-fetch missing subtitles from OpenSubtitles
- Watch mode (auto-process new files as they land)
- Duplicate detection (same film in two resolutions → flag the lower one)
- Manual override: "this is actually X" search box for failed matches

---

## 2. Tech stack

| Layer | Choice | Why |
|---|---|---|
| Shell | **Tauri v2** | Uses the OS WebView, so bundle is ~6–10 MB instead of Electron's 120 MB+ |
| Backend | **Rust** | Fast file I/O, safe, compiles into the Tauri binary |
| Frontend | **React + TypeScript + Tailwind** | Quick to build a clean, modern UI |
| Metadata | **TMDB API** | Free for personal use, excellent film + TV coverage |
| Subtitles | **OpenSubtitles API** | Free tier with daily download limits |
| Local state | **SQLite (rusqlite)** or JSON | Undo log, cache of API responses |

Useful Rust crates: `walkdir`, `regex`, `reqwest`, `serde` / `serde_json`, `tokio`, `image` + `ico`, `rusqlite`, `notify` (for watch mode).

---

## 3. Project structure

```
media-organizer/
├── src/                        # React frontend
│   ├── components/
│   │   ├── FolderPicker.tsx
│   │   ├── ScanResults.tsx     # dry-run preview table
│   │   ├── ProgressPanel.tsx
│   │   └── Settings.tsx
│   ├── lib/
│   │   └── api.ts              # Tauri invoke wrappers
│   ├── App.tsx
│   └── main.tsx
├── src-tauri/
│   ├── src/
│   │   ├── main.rs
│   │   ├── commands.rs         # #[tauri::command] entry points
│   │   ├── scanner.rs          # walk directory, collect media files
│   │   ├── parser.rs           # filename → title/year/season/episode
│   │   ├── tmdb.rs             # metadata lookup + cache
│   │   ├── organiser.rs        # plan + execute moves/renames
│   │   ├── subtitles.rs        # match local subs, fetch missing
│   │   ├── folder_icon.rs      # poster → .ico → desktop.ini
│   │   └── undo.rs             # operation log + rollback
│   ├── Cargo.toml
│   └── tauri.conf.json
└── package.json
```

---

## 4. Output library structure

```
Library/
├── Movies/
│   ├── Inception (2010)/
│   │   ├── Inception (2010).mkv
│   │   ├── Inception (2010).en.srt
│   │   ├── folder.ico
│   │   ├── poster.jpg
│   │   └── desktop.ini
│   └── Dune Part Two (2024)/
└── TV Shows/
    └── Breaking Bad/
        ├── Season 01/
        │   ├── Breaking Bad - S01E01 - Pilot.mkv
        │   └── Breaking Bad - S01E01 - Pilot.en.srt
        ├── folder.ico
        └── desktop.ini
```

Naming should be driven by a **template string** in settings, e.g.
`{title} ({year})/{title} ({year})` and `{show}/Season {season:02}/{show} - S{season:02}E{episode:02} - {episode_title}`.

---

## 5. Core pipeline

1. **Scan** — walk the source folder, collect files with video extensions (`.mkv .mp4 .avi .m4v .mov`). Skip anything under a size threshold (e.g. 50 MB) to avoid samples and trailers.
2. **Parse** — strip release junk from the filename: resolution (1080p, 2160p), source (WEB-DL, BluRay), codec (x264, HEVC), group tags in brackets. What's left is usually the title plus a year. Detect TV with patterns like `S01E02`, `1x02`, `Season 1 Episode 2`.
3. **Identify** — query TMDB with the parsed title + year. Take the top result if the confidence is high; otherwise mark it **needs review** rather than guessing.
4. **Match subtitles** — look for `.srt .ass .sub .vtt` files that share the video's base filename, or sit in a `Subs/` subfolder next to it. Detect language from the suffix (`.en.srt`) or from folder naming.
5. **Plan** — build a list of proposed operations (source path → destination path). **Nothing is executed yet.** This is what the preview screen shows.
6. **Execute** — on confirmation, create folders and move files. Use a move-on-same-drive fast path, copy+verify+delete across drives. Write each operation to the undo log as it completes.
7. **Decorate** — download the poster, convert to multi-size `.ico`, write `desktop.ini`, set the folder's system + read-only attribute so Windows honours it.

---

## 6. Key technical notes

### Folder icons on Windows
Create `folder.ico` inside the target folder, then write:

```ini
[.ShellClassInfo]
IconResource=folder.ico,0
```

Then mark `desktop.ini` as hidden + system, and set the **read-only attribute on the folder itself** — Windows only reads `desktop.ini` when that flag is set. Explorer caches icons aggressively, so include a "refresh icon cache" action in settings for when changes don't appear.

### Filename parsing
Build the regex as a layered cleanup rather than one giant pattern: remove bracketed groups → remove known quality/codec tokens → extract a 4-digit year (1900–2099) → treat everything before the year as the title → replace dots and underscores with spaces. Keep the token list in a config file so it's easy to extend.

### Safety rules (non-negotiable)
- Never delete a source file until the destination copy is verified
- Never overwrite an existing destination — append or skip and report
- Every run writes an undo log entry; keep the last N runs
- Dry-run is the default; executing requires an explicit click

### API keys
TMDB requires a free API key. Let the user paste their own into Settings and store it locally — don't ship yours inside an open-source binary.

---

## 7. Build phases

| Phase | Deliverable | Rough effort |
|---|---|---|
| 0 | Tauri project scaffolded, folder picker, list video files found | Weekend |
| 1 | Filename parser + TMDB lookup, results shown in a table | 1 week |
| 2 | Dry-run plan + execute moves + undo log | 1 week |
| 3 | Subtitle matching | 2–3 days |
| 4 | Poster → folder icon + desktop.ini | 3–4 days |
| 5 | UI polish, settings, error states | 1 week |
| 6 | Optional: OpenSubtitles, watch mode, duplicate detection | Ongoing |

Ship after phase 2. It's already useful at that point.

---

## 8. Risks and gotchas

- **Bad matches on obscure or foreign titles** — mitigate with the "needs review" state and manual search box
- **Explorer icon cache** — icons may not appear until refresh; document this
- **Anime naming** — uses absolute episode numbering, not S/E. Treat as a separate parser path later
- **Long path limits** — Windows caps at 260 chars by default; sanitise and truncate long titles
- **Antivirus false positives** — unsigned binaries that move lots of files can trip heuristics. Expected for unsigned open-source tools

---

## 9. Naming

Something short and searchable — `Cinefold`, `Reelkeeper`, `Shelf`, `Tidyplex`. Check the name isn't taken on GitHub before committing to it.
