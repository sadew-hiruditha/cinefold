//! Tauri command surface: everything the React layer can call.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::catalog::{Catalog, CatalogFile, CatalogPatch, CatalogView};
use crate::folder_icon;
use crate::fsutil;
use crate::organiser::{self, ExecReport, ItemStatus, MediaItem, Plan, ProgressEvent};
use crate::parser::{self, MediaKind, Tokens};
use crate::scanner::{self, SkippedFile};
use crate::settings::Settings;
use crate::subdl::{self, SubdlClient};
use crate::subtitles;
use crate::undo::UndoOp;
use crate::tmdb::{cache_path, Candidate, Identification, TmdbClient};
use crate::undo::{RollbackReport, UndoRun, UndoStore};

pub const PROGRESS_EVENT: &str = "cinefold://progress";

pub struct AppState {
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub settings: Mutex<Settings>,
    pub tokens: Mutex<Arc<Tokens>>,
    pub tmdb: Mutex<Arc<TmdbClient>>,
    pub subdl: Mutex<Arc<SubdlClient>>,
    /// Results of the most recent scan, keyed by item id for quick edits.
    pub items: Mutex<Vec<MediaItem>>,
    pub scan_root: Mutex<Option<PathBuf>>,
    /// Last poster fetched for the Settings preview: (url, decoded image).
    pub preview_poster: Mutex<Option<(String, image::DynamicImage)>>,
    pub catalog: Mutex<Catalog>,
}

impl AppState {
    pub fn new(config_dir: PathBuf, cache_dir: PathBuf) -> Self {
        let settings = Settings::load(&config_dir);
        let tmdb = TmdbClient::new(settings.tmdb_api_key.clone(), cache_path(&cache_dir));

        // A tokens.json dropped into the config folder extends the defaults.
        let tokens = Tokens::load_override(&config_dir.join("tokens.json"))
            .unwrap_or_else(|| Tokens::default());
        let catalog = Catalog::load(&config_dir);
        let subdl = SubdlClient::new(settings.subdl_api_key.clone());

        Self {
            config_dir,
            cache_dir,
            settings: Mutex::new(settings),
            tokens: Mutex::new(Arc::new(tokens)),
            tmdb: Mutex::new(Arc::new(tmdb)),
            subdl: Mutex::new(Arc::new(subdl)),
            items: Mutex::new(Vec::new()),
            scan_root: Mutex::new(None),
            preview_poster: Mutex::new(None),
            catalog: Mutex::new(catalog),
        }
    }

    fn settings_snapshot(&self) -> Settings {
        self.settings
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    fn tmdb_client(&self) -> Arc<TmdbClient> {
        self.tmdb
            .lock()
            .map(|c| Arc::clone(&c))
            .unwrap_or_else(|poisoned| Arc::clone(&poisoned.into_inner()))
    }

    fn subdl_client(&self) -> Arc<SubdlClient> {
        self.subdl
            .lock()
            .map(|c| Arc::clone(&c))
            .unwrap_or_else(|poisoned| Arc::clone(&poisoned.into_inner()))
    }

    fn tokens(&self) -> Arc<Tokens> {
        self.tokens
            .lock()
            .map(|t| Arc::clone(&t))
            .unwrap_or_else(|poisoned| Arc::clone(&poisoned.into_inner()))
    }

    fn undo_store(&self) -> UndoStore {
        UndoStore::new(self.config_dir.join("undo"))
    }
}

fn emit(app: &AppHandle, event: ProgressEvent) {
    let _ = app.emit(PROGRESS_EVENT, event);
}

fn progress(phase: &str, current: usize, total: usize, label: impl Into<String>) -> ProgressEvent {
    ProgressEvent {
        phase: phase.to_string(),
        current,
        total,
        label: label.into(),
        item_id: None,
        error: None,
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings_snapshot()
}

#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<Settings, String> {
    settings.save(&state.config_dir).map_err(|e| e.to_string())?;

    let (key_changed, subdl_changed) = {
        let current = state.settings_snapshot();
        (
            current.tmdb_api_key != settings.tmdb_api_key,
            current.subdl_api_key != settings.subdl_api_key,
        )
    };

    if let Ok(mut guard) = state.settings.lock() {
        *guard = settings.clone();
    }

    if key_changed {
        let client = TmdbClient::new(
            settings.tmdb_api_key.clone(),
            cache_path(&state.cache_dir),
        );
        if let Ok(mut guard) = state.tmdb.lock() {
            *guard = Arc::new(client);
        }
    }

    if subdl_changed {
        if let Ok(mut guard) = state.subdl.lock() {
            *guard = Arc::new(SubdlClient::new(settings.subdl_api_key.clone()));
        }
    }

    Ok(settings)
}

#[tauri::command]
pub async fn validate_subdl_key(key: String) -> Result<bool, String> {
    SubdlClient::new(key)
        .validate_key()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn validate_api_key(state: State<'_, AppState>, key: String) -> Result<bool, String> {
    let cache = cache_path(&state.cache_dir);
    let client = TmdbClient::new(key, cache);
    client.validate_key().await.map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Scanning and identification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub items: Vec<MediaItem>,
    pub skipped: Vec<SkippedFile>,
    pub files_seen: usize,
    pub errors: Vec<String>,
    pub ready: usize,
    pub needs_review: usize,
    pub library_excluded: bool,
    /// Items already sitting at their correct library path.
    pub already_organised: usize,
}

/// Identify a batch with a little concurrency. TMDB tolerates ~50 requests a
/// second; six at a time keeps well inside that while beating a serial loop.
const IDENTIFY_CONCURRENCY: usize = 6;

#[tauri::command]
pub async fn scan_folder(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<ScanResult, String> {
    let root = PathBuf::from(&path);
    if !root.is_dir() {
        return Err(format!("{path} is not a folder"));
    }

    let settings = state.settings_snapshot();
    let tokens = state.tokens();
    let client = state.tmdb_client();

    // --- walk ------------------------------------------------------------
    let walk_app = app.clone();
    let library_root = (!settings.library_root.trim().is_empty())
        .then(|| PathBuf::from(settings.library_root.trim()));
    let scan = scanner::scan(&root, library_root.as_deref(), &settings, move |seen| {
        emit(
            &walk_app,
            progress("scan", seen, 0, format!("{seen} files scanned")),
        );
    });

    // --- pair subtitles ---------------------------------------------------
    let mut subtitle_map = subtitles::match_subtitles(&scan.videos, &scan.subtitles);

    // --- parse ------------------------------------------------------------
    let total = scan.videos.len();
    let mut items: Vec<MediaItem> = Vec::with_capacity(total);

    for (index, video) in scan.videos.iter().enumerate() {
        let parsed = parser::parse_path(&video.path, &tokens);
        let matches = subtitle_map.remove(&video.path).unwrap_or_default();
        let matches = subtitles::filter_languages(&matches, &settings.preferred_languages);

        items.push(MediaItem {
            id: format!("item-{index}"),
            source: video.path.clone(),
            size: video.size,
            parsed,
            identification: Identification::unidentified("not looked up yet"),
            subtitles: matches,
            status: ItemStatus::NeedsReview,
        });
    }

    // --- identify ---------------------------------------------------------
    if !settings.has_api_key() {
        for item in &mut items {
            item.identification =
                Identification::unidentified("add a TMDB API key in Settings to identify files");
        }
    } else {
        let threshold = settings.match_threshold;
        let mut done = 0usize;

        for chunk in items.chunks_mut(IDENTIFY_CONCURRENCY) {
            let mut handles = Vec::with_capacity(chunk.len());
            for item in chunk.iter() {
                let client = Arc::clone(&client);
                let parsed = item.parsed.clone();
                handles.push(tokio::spawn(async move {
                    client.identify(&parsed, threshold).await
                }));
            }

            for (item, handle) in chunk.iter_mut().zip(handles) {
                let identification = match handle.await {
                    Ok(id) => id,
                    Err(err) => Identification::unidentified(format!("lookup failed: {err}")),
                };
                item.status = if identification.needs_review || identification.best.is_none() {
                    ItemStatus::NeedsReview
                } else {
                    ItemStatus::Ready
                };
                item.identification = identification;

                done += 1;
                emit(
                    &app,
                    progress("identify", done, total, item.title()),
                );
            }
        }

        client.save_cache();
    }

    // Files already sitting at their correct library path have nothing to do.
    // Park them as excluded with a clear reason instead of offering a move.
    let mut already_organised = 0usize;
    for item in &mut items {
        if item.status == ItemStatus::Ready && organiser::already_organised(item, &settings) {
            item.status = ItemStatus::Excluded;
            item.identification.reason = "already organised - in the right place".to_string();
            already_organised += 1;
        }
    }

    let ready = items
        .iter()
        .filter(|i| i.status == ItemStatus::Ready)
        .count();
    let needs_review = items
        .iter()
        .filter(|i| i.status == ItemStatus::NeedsReview)
        .count();

    if let Ok(mut guard) = state.items.lock() {
        *guard = items.clone();
    }
    if let Ok(mut guard) = state.scan_root.lock() {
        *guard = Some(root);
    }

    Ok(ScanResult {
        items,
        skipped: scan.skipped,
        files_seen: scan.files_seen,
        errors: scan.errors,
        ready,
        needs_review,
        library_excluded: scan.library_excluded,
        already_organised,
    })
}

/// Promote a file the scanner skipped into the queue: parse, identify, add.
#[tauri::command]
pub async fn include_skipped(
    state: State<'_, AppState>,
    path: String,
) -> Result<MediaItem, String> {
    let source = PathBuf::from(&path);
    if !source.is_file() {
        return Err(format!("{path} is no longer there"));
    }

    let settings = state.settings_snapshot();
    let tokens = state.tokens();
    let size = fsutil::file_size(&source).map_err(|e| e.to_string())?;
    let parsed = parser::parse_path(&source, &tokens);

    let identification = if settings.has_api_key() {
        let client = state.tmdb_client();
        let id = client.identify(&parsed, settings.match_threshold).await;
        client.save_cache();
        id
    } else {
        Identification::unidentified("add a TMDB API key in Settings to identify files")
    };

    let mut items = state.items.lock().map_err(|_| "state is unavailable")?;
    if let Some(existing) = items
        .iter()
        .find(|i| fsutil::paths_equal(&i.source, &source))
    {
        return Ok(existing.clone());
    }

    let status = if identification.needs_review || identification.best.is_none() {
        ItemStatus::NeedsReview
    } else {
        ItemStatus::Ready
    };
    let item = MediaItem {
        id: format!("item-{}", items.len()),
        source,
        size,
        parsed,
        identification,
        subtitles: Vec::new(),
        status,
    };
    items.push(item.clone());
    Ok(item)
}

#[tauri::command]
pub fn get_items(state: State<'_, AppState>) -> Vec<MediaItem> {
    state.items.lock().map(|i| i.clone()).unwrap_or_default()
}

/// Free-text search for the "this is actually X" box.
#[tauri::command]
pub async fn search_titles(
    state: State<'_, AppState>,
    query: String,
    kind: MediaKind,
    year: Option<u16>,
) -> Result<Vec<Candidate>, String> {
    let client = state.tmdb_client();
    let results = client
        .search(kind, &query, year)
        .await
        .map_err(|e| e.to_string())?;
    client.save_cache();
    Ok(results)
}

/// Accept a candidate for an item, manually or from the alternatives list.
#[tauri::command]
pub async fn apply_match(
    state: State<'_, AppState>,
    item_id: String,
    candidate: Candidate,
) -> Result<MediaItem, String> {
    let (season, episode) = {
        let items = state.items.lock().map_err(|_| "state is unavailable")?;
        let item = items
            .iter()
            .find(|i| i.id == item_id)
            .ok_or_else(|| format!("no item {item_id}"))?;
        (item.parsed.season, item.parsed.episode)
    };

    // A TV pick needs its episode title fetched before we can name the file.
    let episode_title = match (candidate.kind, season, episode) {
        (MediaKind::Tv, Some(season), Some(episode)) => {
            let client = state.tmdb_client();
            let title = client.episode_title(candidate.id, season, episode).await;
            client.save_cache();
            title
        }
        _ => None,
    };

    let mut items = state.items.lock().map_err(|_| "state is unavailable")?;
    let item = items
        .iter_mut()
        .find(|i| i.id == item_id)
        .ok_or_else(|| format!("no item {item_id}"))?;

    item.identification = Identification {
        best: Some(candidate),
        alternatives: item.identification.alternatives.clone(),
        confidence: 1.0,
        needs_review: false,
        reason: "confirmed by you".to_string(),
        episode_title,
    };
    item.status = ItemStatus::Ready;
    Ok(item.clone())
}

/// Accept the current best match for each item, fetching episode titles that
/// were skipped while the item was under review. Items with no match are
/// left as they are.
#[tauri::command]
pub async fn approve_items(
    state: State<'_, AppState>,
    item_ids: Vec<String>,
) -> Result<Vec<MediaItem>, String> {
    // Snapshot what needs fetching, without holding the lock across awaits.
    let pending: Vec<(String, u32, u16, u16)> = {
        let items = state.items.lock().map_err(|_| "state is unavailable")?;
        items
            .iter()
            .filter(|i| item_ids.contains(&i.id))
            .filter_map(|i| {
                let best = i.identification.best.as_ref()?;
                if best.kind != MediaKind::Tv || i.identification.episode_title.is_some() {
                    return None;
                }
                Some((i.id.clone(), best.id, i.parsed.season?, i.parsed.episode?))
            })
            .collect()
    };

    let client = state.tmdb_client();
    let mut titles: HashMap<String, String> = HashMap::new();
    for (item_id, tv_id, season, episode) in pending {
        if let Some(title) = client.episode_title(tv_id, season, episode).await {
            titles.insert(item_id, title);
        }
    }
    client.save_cache();

    let mut items = state.items.lock().map_err(|_| "state is unavailable")?;
    let mut updated = Vec::new();
    for item in items.iter_mut().filter(|i| item_ids.contains(&i.id)) {
        if item.identification.best.is_none() {
            continue;
        }
        if let Some(title) = titles.remove(&item.id) {
            item.identification.episode_title = Some(title);
        }
        item.identification.needs_review = false;
        item.identification.reason = "approved by you".to_string();
        item.status = ItemStatus::Ready;
        updated.push(item.clone());
    }
    Ok(updated)
}

/// Fetch a subtitle for one item before it has been organised - written
/// beside the source file so it travels with the video when the run moves it.
/// Only searches; never overwrites a subtitle that is already there.
#[tauri::command]
pub async fn fetch_item_subtitle(
    state: State<'_, AppState>,
    item_id: String,
) -> Result<MediaItem, String> {
    let settings = state.settings_snapshot();
    if !settings.has_subdl_key() {
        return Err("Add a SubDL API key in Settings first.".to_string());
    }

    let (source, parsed, tmdb_id) = {
        let items = state.items.lock().map_err(|_| "state is unavailable")?;
        let item = items
            .iter()
            .find(|i| i.id == item_id)
            .ok_or_else(|| format!("no item {item_id}"))?;
        (
            item.source.clone(),
            item.parsed.clone(),
            item.chosen().map(|c| c.id),
        )
    };
    if !source.is_file() {
        return Err(format!("{} is no longer there", source.display()));
    }

    let client = state.subdl_client();
    let outcome = subdl::fetch_for_video(
        &client,
        &source,
        &parsed,
        tmdb_id,
        &settings.preferred_languages,
        |_| {},
    )
    .await
    .map_err(|e| e.to_string())?;

    if outcome.written.is_empty() {
        let langs = if outcome.missing.is_empty() {
            "the languages you asked for".to_string()
        } else {
            outcome.missing.join(", ")
        };
        return Err(format!("No subtitle found in {langs}."));
    }

    let mut items = state.items.lock().map_err(|_| "state is unavailable")?;
    let item = items
        .iter_mut()
        .find(|i| i.id == item_id)
        .ok_or_else(|| format!("no item {item_id}"))?;

    for path in &outcome.written {
        // Written as "<stem>.<lang>.<ext>" - pull the language back out.
        let language = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.rsplit_once('.'))
            .map(|(_, lang)| lang.to_string());
        item.subtitles.push(subtitles::SubtitleMatch {
            path: path.clone(),
            language,
            forced: false,
            sdh: false,
            score: 1.0,
            reason: "fetched from SubDL".to_string(),
        });
    }
    Ok(item.clone())
}

#[tauri::command]
pub fn set_item_status(
    state: State<'_, AppState>,
    item_id: String,
    status: ItemStatus,
) -> Result<MediaItem, String> {
    let mut items = state.items.lock().map_err(|_| "state is unavailable")?;
    let item = items
        .iter_mut()
        .find(|i| i.id == item_id)
        .ok_or_else(|| format!("no item {item_id}"))?;

    if status == ItemStatus::Ready && item.identification.best.is_none() {
        return Err("this item has no match yet".to_string());
    }
    item.status = status;
    Ok(item.clone())
}

/// Override the season/episode the parser guessed.
#[tauri::command]
pub fn set_item_episode(
    state: State<'_, AppState>,
    item_id: String,
    season: Option<u16>,
    episode: Option<u16>,
) -> Result<MediaItem, String> {
    let mut items = state.items.lock().map_err(|_| "state is unavailable")?;
    let item = items
        .iter_mut()
        .find(|i| i.id == item_id)
        .ok_or_else(|| format!("no item {item_id}"))?;
    item.parsed.season = season;
    item.parsed.episode = episode;
    Ok(item.clone())
}

// ---------------------------------------------------------------------------
// Planning and execution
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn build_plan(state: State<'_, AppState>) -> Plan {
    let settings = state.settings_snapshot();
    let items = state.items.lock().map(|i| i.clone()).unwrap_or_default();
    match state.catalog.lock() {
        Ok(catalog) => organiser::build_plan(&items, &settings, &catalog),
        Err(_) => organiser::build_plan(&items, &settings, &Catalog::default()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecSummary {
    pub run_id: String,
    pub moved: usize,
    pub copied: usize,
    pub skipped: usize,
    pub decorated: usize,
    pub subtitles: usize,
    pub failures: Vec<String>,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub async fn execute_plan(
    app: AppHandle,
    state: State<'_, AppState>,
    plan: Plan,
) -> Result<ExecSummary, String> {
    let settings = state.settings_snapshot();
    if settings.library_root.trim().is_empty() {
        return Err("Set a library folder in Settings first.".to_string());
    }

    let scan_root = state
        .scan_root
        .lock()
        .ok()
        .and_then(|r| r.clone())
        .unwrap_or_else(|| PathBuf::from("."));

    let store = state.undo_store();
    let mut journal = store.begin(&scan_root, Path::new(&settings.library_root));

    // --- posters first (async), so decoration can run inside the move loop --
    // Keyed by folder: a series has one show folder shared by every episode.
    let mut posters: HashMap<PathBuf, Option<Vec<u8>>> = HashMap::new();
    if settings.set_folder_icons {
        let client = state.tmdb_client();
        let wanted: Vec<(PathBuf, Option<String>, String)> = plan
            .entries
            .iter()
            .filter(|e| e.enabled)
            .map(|e| (e.decorate_dir.clone(), e.poster_url.clone(), e.title.clone()))
            .collect();
        let total = wanted.len();
        for (index, (folder, url, title)) in wanted.into_iter().enumerate() {
            if posters.contains_key(&folder) {
                continue;
            }
            emit(&app, progress("decorate", index + 1, total, format!("poster for {title}")));
            let bytes = match url {
                Some(url) => match client.download(&url).await {
                    Ok(bytes) => Some(bytes),
                    Err(err) => {
                        eprintln!("poster download failed for {title}: {err}");
                        None
                    }
                },
                None => None,
            };
            posters.insert(folder, bytes);
        }
        client.save_cache();
    }

    // --- move files (blocking, off the async executor) ---------------------
    let report: ExecReport = {
        let app = app.clone();
        let settings = settings.clone();
        let plan = &plan;
        let journal = &mut journal;
        tokio::task::block_in_place(move || {
            let mut done: HashSet<PathBuf> = HashSet::new();
            organiser::execute(
                plan,
                &settings,
                journal,
                |event| {
                    let _ = app.emit(PROGRESS_EVENT, event);
                },
                |entry, journal| {
                    if !done.insert(entry.decorate_dir.clone()) {
                        return Ok(false);
                    }
                    let poster = posters
                        .get(&entry.decorate_dir)
                        .and_then(|p| p.as_deref());
                    let opts = folder_icon::IconOptions::for_kind(&settings, entry.kind);
                    folder_icon::decorate(&entry.decorate_dir, &entry.title, poster, &opts, journal)
                        .map(|_| true)
                        .map_err(|e| e.to_string())
                },
            )
        })
    };

    if report.decorated > 0 {
        fsutil::refresh_icon_cache();
    }
    let decorated = report.decorated;
    let mut warnings = report.decorate_failures.clone();

    // --- record in the collection --------------------------------------------
    if !report.placed.is_empty() {
        if let Err(err) = record_placed(&state, &report.placed).await {
            warnings.push(format!("collection not updated: {err}"));
        }
    }

    // --- subtitles for anything that arrived without one ------------------------
    let mut subtitles = 0usize;
    if settings.fetch_subtitles && settings.has_subdl_key() {
        let client = state.subdl_client();
        let items = state.items.lock().map(|i| i.clone()).unwrap_or_default();
        let todo: Vec<&organiser::PlacedFile> = report
            .placed
            .iter()
            .filter(|p| !p.has_subtitle && !subdl::has_subtitle_beside(&p.destination))
            .collect();
        let total = todo.len();

        for (index, placed) in todo.into_iter().enumerate() {
            let Some(item) = items.iter().find(|i| i.id == placed.item_id) else {
                continue;
            };
            emit(&app, progress("subtitles", index + 1, total, item.title()));

            let tmdb_id = item.chosen().map(|c| c.id);
            let result = subdl::fetch_for_video(
                &client,
                &placed.destination,
                &item.parsed,
                tmdb_id,
                &settings.preferred_languages,
                |path| {
                    journal.record(UndoOp::WroteFile {
                        path: path.to_path_buf(),
                    });
                },
            )
            .await;

            match result {
                Ok(outcome) => subtitles += outcome.written.len(),
                Err(err) => {
                    let message = err.to_string();
                    let fatal = message.contains("limit") || message.contains("API key");
                    warnings.push(format!("subtitles for {}: {message}", item.title()));
                    if fatal {
                        break;
                    }
                }
            }
        }
    }

    let run = journal.finish().map_err(|e| e.to_string())?;
    store.prune(settings.keep_undo_runs.max(1));

    emit(&app, progress("done", 1, 1, "finished"));

    Ok(ExecSummary {
        run_id: run.id,
        moved: report.moved,
        copied: report.copied,
        skipped: report.skipped,
        decorated,
        subtitles,
        failures: report.failures,
        warnings,
    })
}

/// Fold the files a run placed into the collection, one TMDB lookup per title.
async fn record_placed(
    state: &State<'_, AppState>,
    placed: &[organiser::PlacedFile],
) -> Result<(), String> {
    let items = state.items.lock().map_err(|_| "state is unavailable")?.clone();
    let client = state.tmdb_client();

    // Group by the identified title so a season lands as one entry.
    let mut groups: HashMap<String, (Candidate, PathBuf, Vec<CatalogFile>)> = HashMap::new();
    for file in placed {
        let Some(item) = items.iter().find(|i| i.id == file.item_id) else {
            continue;
        };
        let Some(candidate) = item.chosen() else {
            continue;
        };
        let key = crate::catalog::CatalogEntry::key_for(candidate.kind, candidate.id);
        let group = groups
            .entry(key)
            .or_insert_with(|| (candidate.clone(), file.folder.clone(), Vec::new()));
        group.2.push(CatalogFile {
            path: file.destination.clone(),
            season: item.parsed.season,
            episode: item.parsed.episode,
            episode_title: item.identification.episode_title.clone(),
            resolution: item.parsed.resolution.clone(),
            size: item.size,
        });
    }

    let mut with_details = Vec::with_capacity(groups.len());
    for (_, (candidate, folder, files)) in groups {
        let details = client.details(candidate.kind, candidate.id).await;
        with_details.push((candidate, details, folder, files));
    }
    client.save_cache();

    let mut catalog = state.catalog.lock().map_err(|_| "state is unavailable")?;
    for (candidate, details, folder, files) in with_details {
        catalog.upsert(&candidate, details.as_ref(), folder, files);
    }
    catalog.save(&state.config_dir).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Collection
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn catalog_list(state: State<'_, AppState>) -> Vec<CatalogView> {
    state
        .catalog
        .lock()
        .map(|c| c.views())
        .unwrap_or_default()
}

#[tauri::command]
pub fn catalog_update(
    state: State<'_, AppState>,
    key: String,
    patch: CatalogPatch,
) -> Result<CatalogView, String> {
    let mut catalog = state.catalog.lock().map_err(|_| "state is unavailable")?;
    catalog
        .apply(&key, patch)
        .ok_or_else(|| format!("no collection entry {key}"))?;
    catalog.save(&state.config_dir).map_err(|e| e.to_string())?;
    catalog
        .views()
        .into_iter()
        .find(|v| v.entry.key == key)
        .ok_or_else(|| "entry vanished".to_string())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleFetchReport {
    pub added: usize,
    pub already_had: usize,
    pub failures: Vec<String>,
}

/// Fetch subtitles for every file of one collection entry that lacks them.
#[tauri::command]
pub async fn catalog_fetch_subtitles(
    app: AppHandle,
    state: State<'_, AppState>,
    key: String,
) -> Result<SubtitleFetchReport, String> {
    let settings = state.settings_snapshot();
    if !settings.has_subdl_key() {
        return Err("Add a SubDL API key in Settings first.".to_string());
    }
    let entry = state
        .catalog
        .lock()
        .map_err(|_| "state is unavailable")?
        .entries
        .get(&key)
        .cloned()
        .ok_or_else(|| format!("no collection entry {key}"))?;

    let client = state.subdl_client();
    let tokens = state.tokens();
    let mut report = SubtitleFetchReport::default();
    let total = entry.files.len();

    for (index, file) in entry.files.iter().enumerate() {
        if !file.path.is_file() {
            continue;
        }
        if subdl::has_subtitle_beside(&file.path) {
            report.already_had += 1;
            continue;
        }
        emit(
            &app,
            progress(
                "subtitles",
                index + 1,
                total,
                file.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            ),
        );

        // The organised name is clean, so parse it for season/episode and
        // patch in what the catalog remembers about the release.
        let mut parsed = parser::parse_path(&file.path, &tokens);
        parsed.kind = entry.kind;
        parsed.season = file.season.or(parsed.season);
        parsed.episode = file.episode.or(parsed.episode);
        parsed.resolution = file.resolution.clone().or(parsed.resolution);

        match subdl::fetch_for_video(
            &client,
            &file.path,
            &parsed,
            Some(entry.tmdb_id),
            &settings.preferred_languages,
            |_| {},
        )
        .await
        {
            Ok(outcome) => {
                report.added += outcome.written.len();
                if outcome.written.is_empty() {
                    report.failures.push(format!(
                        "{}: nothing found in {}",
                        file.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                        if outcome.missing.is_empty() { "any language".to_string() } else { outcome.missing.join(", ") }
                    ));
                }
            }
            Err(err) => {
                let message = err.to_string();
                report.failures.push(message.clone());
                if message.contains("limit") || message.contains("API key") {
                    break;
                }
            }
        }
    }

    emit(&app, progress("done", 1, 1, "finished"));
    Ok(report)
}

#[tauri::command]
pub fn catalog_remove(state: State<'_, AppState>, key: String) -> Result<(), String> {
    let mut catalog = state.catalog.lock().map_err(|_| "state is unavailable")?;
    catalog.remove(&key);
    catalog.save(&state.config_dir).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub added: usize,
    pub files: usize,
    pub unmatched: Vec<String>,
}

/// A batch of files that the folder names say belong to one title.
struct ImportGroup {
    parsed: parser::ParsedName,
    folder: PathBuf,
    files: Vec<(PathBuf, parser::ParsedName, u64)>,
}

/// Walk the organised library and add everything it can identify. Meant for
/// libraries built before the collection existed, or by hand.
#[tauri::command]
pub async fn catalog_import_library(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ImportReport, String> {
    let settings = state.settings_snapshot();
    let root = PathBuf::from(settings.library_root.trim());
    if !root.is_dir() {
        return Err("Set a library folder in Settings first.".to_string());
    }
    if !settings.has_api_key() {
        return Err("Add a TMDB API key in Settings first.".to_string());
    }

    let tokens = state.tokens();
    let client = state.tmdb_client();

    let walk_app = app.clone();
    let scan = scanner::scan(&root, None, &settings, move |seen| {
        emit(&walk_app, progress("scan", seen, 0, format!("{seen} files scanned")));
    });

    let mut groups: HashMap<String, ImportGroup> = HashMap::new();
    for video in &scan.videos {
        let parsed = parser::parse_path(&video.path, &tokens);
        if !parsed.looks_usable() {
            continue;
        }
        let parent = video
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let folder = match parsed.kind {
            MediaKind::Movie => parent,
            MediaKind::Tv => {
                // Episodes usually sit in Show/Season NN/; the show is one up.
                let parent_name = parent
                    .file_name()
                    .map(|n| n.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if parent_name.starts_with("season") || parent_name.starts_with("specials") {
                    parent.parent().map(|p| p.to_path_buf()).unwrap_or(parent)
                } else {
                    parent
                }
            }
        };
        let key = format!(
            "{:?}|{}|{:?}",
            parsed.kind,
            parsed.title.to_lowercase(),
            parsed.year
        );
        groups
            .entry(key)
            .or_insert_with(|| ImportGroup {
                parsed: parsed.clone(),
                folder,
                files: Vec::new(),
            })
            .files
            .push((video.path.clone(), parsed, video.size));
    }

    let total = groups.len();
    let mut report = ImportReport::default();
    let mut resolved = Vec::new();

    for (index, group) in groups.into_values().enumerate() {
        emit(
            &app,
            progress("identify", index + 1, total, group.parsed.title.clone()),
        );
        let identification = client
            .identify(&group.parsed, settings.match_threshold)
            .await;
        let Some(candidate) = identification.best else {
            report.unmatched.push(group.parsed.title.clone());
            continue;
        };
        if identification.needs_review {
            report.unmatched.push(format!(
                "{} ({})",
                group.parsed.title, identification.reason
            ));
            continue;
        }

        let details = client.details(candidate.kind, candidate.id).await;
        let files: Vec<CatalogFile> = group
            .files
            .into_iter()
            .map(|(path, parsed, size)| CatalogFile {
                path,
                season: parsed.season,
                episode: parsed.episode,
                episode_title: None,
                resolution: parsed.resolution,
                size,
            })
            .collect();
        report.files += files.len();
        report.added += 1;
        resolved.push((candidate, details, group.folder, files));
    }
    client.save_cache();

    let mut catalog = state.catalog.lock().map_err(|_| "state is unavailable")?;
    for (candidate, details, folder, files) in resolved {
        catalog.upsert(&candidate, details.as_ref(), folder, files);
    }
    catalog.save(&state.config_dir).map_err(|e| e.to_string())?;

    emit(&app, progress("done", 1, 1, "finished"));
    Ok(report)
}

// ---------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_undo_runs(state: State<'_, AppState>) -> Vec<UndoRun> {
    state.undo_store().list()
}

#[tauri::command]
pub fn undo_run(state: State<'_, AppState>, run_id: String) -> Result<RollbackReport, String> {
    let report = state
        .undo_store()
        .rollback(&run_id)
        .map_err(|e| e.to_string())?;
    fsutil::refresh_icon_cache();
    Ok(report)
}

#[tauri::command]
pub fn refresh_icon_cache() {
    fsutil::refresh_icon_cache();
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReapplyReport {
    pub updated: usize,
    pub failures: Vec<String>,
}

/// Walk the library and regenerate every folder icon with the saved settings.
///
/// Used after changing the icon style so folders organised earlier match.
#[tauri::command]
pub async fn reapply_library_icons(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ReapplyReport, String> {
    let settings = state.settings_snapshot();
    let root = PathBuf::from(settings.library_root.trim());
    if !root.is_dir() {
        return Err("Set a library folder in Settings first.".to_string());
    }

    let folders = folder_icon::decorated_folders(&root);
    let total = folders.len();

    let report = tokio::task::block_in_place(|| {
        let mut report = ReapplyReport::default();
        for (index, folder) in folders.iter().enumerate() {
            let label = folder
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            emit(&app, progress("decorate", index + 1, total, label.clone()));

            let opts = folder_icon::IconOptions::for_kind(
                &settings,
                folder_icon::guess_kind(folder),
            );
            match folder_icon::reapply(folder, &opts) {
                Ok(()) => report.updated += 1,
                Err(err) => report.failures.push(format!("{label}: {err}")),
            }
        }
        report
    });

    if report.updated > 0 {
        fsutil::refresh_icon_cache();
    }
    emit(&app, progress("done", 1, 1, "finished"));
    Ok(report)
}

/// Render the folder icon at a few sizes for the Settings preview.
///
/// Takes the unsaved draft settings so the preview tracks the sliders live.
/// The sample poster is downloaded once and kept in memory.
#[tauri::command]
pub async fn preview_icon(
    state: State<'_, AppState>,
    settings: Settings,
    poster_url: Option<String>,
    kind: Option<MediaKind>,
) -> Result<Vec<String>, String> {
    let opts = folder_icon::IconOptions::for_kind(&settings, kind.unwrap_or(MediaKind::Movie));

    let poster = match poster_url {
        Some(url) => {
            let cached = state
                .preview_poster
                .lock()
                .ok()
                .and_then(|guard| guard.as_ref().filter(|(u, _)| *u == url).map(|(_, img)| img.clone()));

            match cached {
                Some(img) => img,
                None => {
                    let bytes = state
                        .tmdb_client()
                        .download(&url)
                        .await
                        .map_err(|e| e.to_string())?;
                    let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
                    if let Ok(mut guard) = state.preview_poster.lock() {
                        *guard = Some((url, img.clone()));
                    }
                    img
                }
            }
        }
        None => folder_icon::placeholder_poster(),
    };

    [128u32, 48, 16]
        .iter()
        .map(|size| folder_icon::preview_png(Some(&poster), *size, &opts).map_err(|e| e.to_string()))
        .collect()
}

/// Where settings, the undo log and the TMDB cache live, for the about box.
#[tauri::command]
pub fn app_paths(state: State<'_, AppState>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    out.insert(
        "config".to_string(),
        state.config_dir.to_string_lossy().to_string(),
    );
    out.insert(
        "cache".to_string(),
        state.cache_dir.to_string_lossy().to_string(),
    );
    out.insert(
        "undo".to_string(),
        state.config_dir.join("undo").to_string_lossy().to_string(),
    );
    out
}
