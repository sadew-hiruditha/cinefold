# Cinefold

A small, fast Windows desktop tool that scans a messy downloads folder,
identifies films and TV series against TMDB, organises them into a clean
library with matched subtitles, and sets the poster as the folder icon.

Everything is previewed before it runs, and every run can be undone.

## Prerequisites

| Tool | Why | Install |
|---|---|---|
| Node 18+ | Frontend build | https://nodejs.org |
| Rust (stable) | Backend | https://rustup.rs — run the installer, then restart your terminal |
| Visual Studio Build Tools | Rust needs the MSVC linker | https://visualstudio.microsoft.com/visual-cpp-build-tools/ — tick **Desktop development with C++** |
| WebView2 | Renders the UI | Already part of Windows 10/11 |
| TMDB API key | Identifies titles | Free at https://www.themoviedb.org/settings/api — paste it into Settings inside the app |

## Run it

```powershell
npm install
npm run tauri:dev
```

The first Rust build takes a few minutes; later builds are incremental.

## Build an installer

```powershell
npm run tauri:build
```

Produces an NSIS installer under `src-tauri/target/release/bundle/nsis/`.

## Tests

```powershell
cd src-tauri
cargo test
```

Covers the filename parser, subtitle pairing, template rendering, path
sanitisation, icon generation and the undo journal.

## How it works

1. **Scan** — walks the source folder for video files, skipping samples,
   trailers and anything under the size threshold. Subtitles are collected
   alongside.
2. **Parse** — strips release junk from each filename (resolution, source,
   codec, group tags) and pulls out the title, year and `S01E02` markers. The
   vocabulary lives in `src-tauri/tokens.json`; drop a `tokens.json` with extra
   entries into the app's config folder to extend it without rebuilding.
3. **Identify** — queries TMDB. A result is only accepted automatically when
   the title similarity clears the configured threshold and no other candidate
   is nearly as good; everything else lands in **Needs review**, where you can
   search and pick the right title.
4. **Match subtitles** — pairs `.srt`/`.ass`/… files by shared name, `Subs/`
   folder, or being the only video in a release folder, and reads the language
   from the suffix.
5. **Plan** — renders the naming templates into a list of proposed moves. This
   is the preview screen. Nothing is touched.
6. **Execute** — renames on the same drive, copy → verify → delete across
   drives. Each step is journalled before the next begins.
7. **Decorate** — downloads the poster, builds a multi-size `folder.ico`,
   writes a hidden `desktop.ini`, and sets the folder attributes Windows needs
   to honour it.

## Safety rules

- A source file is never deleted until the destination is verified
  (size always; SHA-256 when crossing drives, unless disabled).
- An existing destination is never overwritten — the new file gets ` (2)`.
- Every run is journalled to `%APPDATA%\com.cinefold.app\undo\` and can be
  rolled back from **History**.
- Preview is the default. Moving requires a deliberate click.

## Layout

```
src/                    React + TypeScript + Tailwind
  components/           FolderPicker, ScanResults, MatchDialog, PlanPreview,
                        ProgressPanel, Settings, UndoHistory
  lib/api.ts            Typed wrappers around the Rust commands
src-tauri/src/
  scanner.rs            Directory walk
  parser.rs             Filename → title / year / season / episode
  tmdb.rs               Lookup, scoring, on-disk cache
  subtitles.rs          Subtitle pairing and language detection
  organiser.rs          Template rendering, planning, execution
  folder_icon.rs        Poster → .ico → desktop.ini
  undo.rs               Journal and rollback
  fsutil.rs             Long paths, attributes, sanitisation
  commands.rs           Tauri command surface
```

## Known limits

- Anime with absolute episode numbering is detected but not yet mapped to
  TMDB seasons.
- Explorer caches icons aggressively. If a folder still shows the stock icon,
  use **Settings → Refresh icon cache** or sign out and back in.
- The binary is unsigned, so SmartScreen will warn on first launch.
