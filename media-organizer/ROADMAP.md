# Cinefold — Roadmap

Where the app is, and what takes it to the next level. Ordered by payoff per
effort, with the dependencies between phases called out.

## Where we are (2026-09-13)

Done and verified on real files:

- Scan → parse → TMDB identify → subtitle pairing → dry-run preview →
  move/copy with verification → journalled undo
- Review queue with manual search and season/episode correction
- Duplicate detection (keeps the higher resolution)
- Folder icons: three styles, fill mode, border/corner/colour, live preview,
  re-apply to an existing library
- 48 unit tests on the Rust side

Not yet done from the original plan: OpenSubtitles fetch, watch mode.

Not in any repo yet. **That is the first thing to fix.**

---

## Phase 1 — Foundations (½ day)

Everything after this benefits from it, so it goes first.

| Item | Why |
|---|---|
| `git init`, first commit, push to GitHub | No history means no safe experimentation and no undo for *code* |
| GitHub Actions: `cargo test`, `tsc --noEmit`, `cargo clippy`, `tauri build` on push | Catches regressions; produces an installer artifact per commit |
| File logging (`tracing` + rolling file in the app data dir) and a **Copy diagnostics** button in Settings | Without this, the first bug report from someone else is "it didn't work" |
| `CHANGELOG.md` + version bump script | Needed the moment there is a second user |

---

## Phase 2 — Matching accuracy (2–3 days)

The app is only as good as its guesses. Every item here removes a category of
"Needs review".

| Item | What it fixes |
|---|---|
| **Alternative titles** endpoint | Non-English titles, AKA names, regional spellings match on the first try instead of via the AKA split |
| **Find by IMDb / TVDB ID** | A filename carrying `tt1375666` or `[tvdb-12345]` is a certain match, no scoring needed |
| **Multi-search** fallback | When the parser cannot tell film from series, ask TMDB rather than guessing |
| **Remember corrections** | Once you pick "Silo (2023)" for a `Silo.S01E01` file, every future `Silo.SxxExx` skips review. A small local JSON map keyed on the cleaned title |
| **Anime path** | Use TMDB episode groups (absolute order) to map `[Group] Show - 37` to the right season/episode |
| **Parser corpus test** | A `fixtures/filenames.txt` with 200+ real release names and their expected parse, run as one test. Catches regressions every time the token list changes |

Dependency: none. Highest value per hour of anything on this list.

---

## Phase 3 — Media-centre compatibility (2–3 days)

Makes the library something Jellyfin, Plex, Kodi and Emby read perfectly,
which is what most people organising a media folder actually want.

| Item | Output |
|---|---|
| **NFO files** | `movie.nfo` / `tvshow.nfo` / per-episode `.nfo` in Kodi's XML format, with TMDB and IMDb IDs. Media centres then skip their own (worse) matching |
| **Backdrops and logos** | `fanart.jpg` and `clearlogo.png` beside `poster.jpg`, straight from TMDB's image lists |
| **Season posters** | `Season 01/folder.ico` + `season01-poster.jpg` so season folders get their own art |
| **Collections** | Optional `Movies/Dark Knight Collection/The Dark Knight (2008)/…` grouping with a collection poster on the parent |
| **Extras** | Route trailers and featurettes into `Extras/` under the film instead of skipping them |
| **Multi-version** | `Blade Runner (1982) - Final Cut.mkv` next to `- Theatrical.mkv` using the `{edition}` placeholder, rather than " (2)" |

Dependency: Phase 2 (IDs in NFOs need reliable matches).

---

## Phase 4 — Library health check (1–2 days)

Today the app only looks at the *source*. Pointing it at the *library* opens up
maintenance.

| Item | What it does |
|---|---|
| **Scan the library itself** | Walk the organised tree, identify each folder from its name, report what is missing: icon, poster, NFO, subtitles, episodes in a season |
| **Fix in place** | Fetch missing art / NFOs without moving anything |
| **Rename to current template** | If the naming template changed, preview and apply renames inside the library, journalled like any run |
| **Orphan detection** | Folders with no video, `.tmp` leftovers, stale `desktop.ini` |

Dependency: Phase 3 (needs the NFO/art writers to have something to fix).

---

## Phase 5 — Automation (2–3 days)

| Item | Behaviour |
|---|---|
| **Watch mode** | `notify` crate on the source folder; new files debounce for ~30 s (downloads finish), then process. Above-threshold matches go straight through; the rest queue for review |
| **System tray** | Minimise to tray, badge count of items awaiting review, "Pause watching" |
| **Windows toast** | "Organised *Dune Part Two* — 1 file" / "3 files need review" |
| **Run at startup** | Optional, off by default |
| **OpenSubtitles** | For items with no local subtitle: search by file hash + language prefs, download, name to match. Respect the free-tier daily cap and show remaining quota |

Dependency: Phase 1 logging (unattended runs must leave a trail). Watch mode
must also honour the safety rules exactly as manual runs do.

---

## Phase 6 — Robustness (1–2 days)

| Item | Scenario it handles |
|---|---|
| **Crash-safe resume** | App or PC dies mid-run. On next start, detect the unfinished journal and offer *Resume* or *Roll back* |
| **Locked-file handling** | Video open in a player: retry with backoff, then skip and report, never fail the whole run |
| **UNC / network paths** | `\\NAS\media` sources and libraries: `same_volume` must compare server+share, not drive letters |
| **Throughput + ETA** | Cross-drive copies of 50 GB need a bytes/s readout and time remaining, not a bar that sits still |
| **Integration test suite** | Build a temp source tree, run the whole pipeline against a mocked TMDB, assert the on-disk result, roll it back, assert the tree is restored byte-for-byte |

---

## Phase 7 — UX polish (2–3 days)

| Item |
|---|
| Destination shown on every row in results, before opening the preview |
| Bulk select in review: exclude / include / apply match to all selected episodes of a show |
| Keyboard: `J`/`K` to move, `Enter` to identify, `X` to exclude, `Ctrl+Enter` to preview |
| Drag a folder onto the window to scan it |
| Right-click / buttons: *Reveal in Explorer*, *Open on TMDB* |
| Overview and runtime in the match dialog to tell similar titles apart |
| Light theme following the system setting |
| First-run wizard: API key → library folder → icon style, three screens |
| Remember last source folder and window size |

---

## Phase 8 — Distribution (1–2 days, then ongoing)

| Item | Why |
|---|---|
| **Auto-updater** (`tauri-plugin-updater`) with signed manifests | Users never re-download an installer |
| **Code signing** | Removes the SmartScreen warning. An OV cert costs money; Azure Trusted Signing is the cheaper route |
| **Portable build** | Single `.exe` alongside the installer for people who cannot install |
| **Website / README with a 20-second GIF** | The organise → preview → done loop is the pitch |
| **Issue templates** that ask for the diagnostics bundle from Phase 1 | |

---

## Suggested order

```
1 Foundations ─┬─▶ 2 Matching ──▶ 3 Media-centre ──▶ 4 Health check
               │
               ├─▶ 6 Robustness
               │
               └─▶ 5 Automation ──▶ 8 Distribution
                                         ▲
                            7 Polish ────┘
```

Realistic pace at a few evenings a week: Phases 1–3 in the first month, 4–6 in
the second, 7–8 in the third. Ship a public 0.2 after Phase 3 — that is the
point where it does something Plex and Jellyfin users cannot easily get
elsewhere (clean library + folder icons + NFOs in one pass).

## Parked (decided, not yet built)

**Explorer icon-cache hardening** — for folders whose cached thumbnail is
already stale, nothing short of clearing the cache works. Three-part fix, all
agreed on 2026-09-13, to be built on request:

1. After decorating, rename the folder to a temp name and back so Explorer
   treats it as new; set `SYSTEM` alongside `READONLY`; bump the folder mtime.
2. A **Refresh icons** button on the Organise page (runs re-apply).
3. A **Restart Explorer** button in Settings as the guaranteed fallback.

## Explicitly not planned

- Mac / Linux builds — folder icons are the headline feature and they are
  Windows-specific. Revisit only if the NFO/media-centre side becomes the
  main draw.
- Music, photos, audiobooks — different metadata sources, different naming
  conventions, would dilute the parser.
- A cloud or account component — the whole point is that files never leave
  the machine.
Search & identification
✅ Search movies / TV — by title, with year filter
Multi search — one query across movies, TV and people at once (would let us skip guessing film-vs-series)
Find by external ID — look up by IMDb ID (tt1375666), TVDB ID, etc. Useful when a filename contains an IMDb tag like [tt1375666]
Alternative titles — every regional/AKA title for a film or show (would improve matching non-English releases like the Tangerines one)
Translations — titles and overviews in each language
Movie details
Title, original title, tagline, overview, runtime, release date, status
Genres, production companies, countries, spoken languages
Budget, revenue, rating (vote_average), popularity
Release dates by country with certification (PG-13, 15, etc.)
Belongs-to-collection (e.g. "The Dark Knight Collection")
✅ Poster path; also backdrop (wide 16:9 art) and logo images
Videos (trailer YouTube keys)
Credits (cast + crew), keywords, reviews
Similar / recommended movies
Watch providers (where it's streaming, by country)
TV series details
Name, overview, first/last air date, status (Returning / Ended)
Number of seasons and episodes, episode runtime
Networks, created-by, genres, content ratings
Poster, backdrop, logos
Season details — all episodes in a season with names, air dates, overviews, stills
✅ Episode details — name, overview, air date, still image, guest stars
Episode groups — alternate orderings (DVD order, absolute order — relevant for anime)
External IDs (IMDb, TVDB) per show / season / episode
Images
Poster, backdrop, logo, still (episode frame), profile (person) — each in multiple sizes (w92 → original)
Per-language variants (e.g. a textless poster or an English-logo version)
Collections
Name, overview, poster/backdrop, and the ordered list of films in it (e.g. all MCU or all Harry Potter)
People
Name, biography, birthday, profile photo, combined credits
Discovery & lists
Discover — filter by genre, year, rating, language, provider, etc.
Trending, popular, top rated, now playing, upcoming
Genre list, certifications list, languages, countries, configuration (image base URLs, sizes)
Account (needs user login, not just an API key)
Ratings, favourites, watchlist, custom lists
Worth adding to Cinefold
Feature	What it enables
Alternative titles	Match "Pokssak Sogatsuda" straight away instead of splitting on AKA
Find by IMDb ID	Instant, certain match when the filename carries tt…
Collections	Optional Movies/The Dark Knight Collection/… grouping, or one icon per franchise
Backdrop / logo images	A wide-art icon style, or a backdrop.jpg alongside the poster for media centres
Genres / certification	Template placeholders like {genre} or {rating} for naming
Season posters	Per-season folder icons instead of only the show folder
Episode groups	Proper anime handling — map absolute episode numbers to seasons
Overview + runtime	Show in the review dialog to help disambiguate
Textless posters	A "no title text" preference for cleaner icons
If any of those appeal, alternative titles and IMDb-ID lookup are the cheapest wins for matching accuracy; season posters and backdrops are the cheapest for looks.