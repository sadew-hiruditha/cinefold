/**
 * Typed wrappers around the Rust commands.
 *
 * Field names mirror the serde `camelCase` renaming on the Rust side, so these
 * interfaces are the single source of truth for the shape of the bridge.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type MediaKind = "movie" | "tv";
export type ItemStatus = "ready" | "needsReview" | "excluded";
export type TitleSource = "filename" | "folder";
export type OpKind = "video" | "subtitle";
export type IconStyle = "card" | "folder" | "poster";

export interface ParsedName {
  raw: string;
  title: string;
  year: number | null;
  season: number | null;
  episode: number | null;
  extraEpisodes: number[];
  absoluteEpisode: number | null;
  kind: MediaKind;
  edition: string | null;
  resolution: string | null;
  source: string | null;
  codec: string | null;
  group: string | null;
  titleSource: TitleSource;
}

export interface Candidate {
  id: number;
  title: string;
  originalTitle: string;
  year: number | null;
  overview: string;
  posterPath: string | null;
  backdropPath: string | null;
  popularity: number;
  voteAverage: number;
  kind: MediaKind;
  score: number;
}

export interface Identification {
  best: Candidate | null;
  alternatives: Candidate[];
  confidence: number;
  needsReview: boolean;
  reason: string;
  episodeTitle: string | null;
}

export interface SubtitleMatch {
  path: string;
  language: string | null;
  forced: boolean;
  sdh: boolean;
  score: number;
  reason: string;
}

export interface MediaItem {
  id: string;
  source: string;
  size: number;
  parsed: ParsedName;
  identification: Identification;
  subtitles: SubtitleMatch[];
  status: ItemStatus;
}

export interface SkippedFile {
  path: string;
  size: number;
  reason: string;
}

export interface ScanResult {
  items: MediaItem[];
  skipped: SkippedFile[];
  filesSeen: number;
  errors: string[];
  ready: number;
  needsReview: number;
  /** The library sits inside the source and was skipped by the walk. */
  libraryExcluded: boolean;
  /** Files found already at their correct library path (parked as excluded). */
  alreadyOrganised: number;
}

export interface PlannedOp {
  kind: OpKind;
  source: string;
  destination: string;
  size: number;
  note: string | null;
}

export interface PlanEntry {
  itemId: string;
  title: string;
  kind: MediaKind;
  decorateDir: string;
  destinationDir: string;
  ops: PlannedOp[];
  posterUrl: string | null;
  warnings: string[];
  enabled: boolean;
  duplicateOf: string | null;
}

export interface Plan {
  libraryRoot: string;
  entries: PlanEntry[];
  warnings: string[];
  totalBytes: number;
  fileCount: number;
  crossVolume: boolean;
}

export interface Settings {
  libraryRoot: string;
  movieTemplate: string;
  tvTemplate: string;
  tmdbApiKey: string;
  subdlApiKey: string;
  fetchSubtitles: boolean;
  minFileSizeMb: number;
  videoExtensions: string[];
  subtitleExtensions: string[];
  matchThreshold: number;
  preferredLanguages: string[];
  setFolderIcons: boolean;
  folderTint: string;
  tvTint: string;
  separateTvTint: boolean;
  iconStyle: IconStyle;
  /** Percent of icon size, 0-20. */
  iconBorder: number;
  /** Percent of icon size, 0-30. */
  iconCorner: number;
  /** Crop the poster to fill the shape instead of fitting 2:3. */
  iconFill: boolean;
  copyInsteadOfMove: boolean;
  deepVerify: boolean;
  keepUndoRuns: number;
  skipSamples: boolean;
}

export interface ExecSummary {
  runId: string;
  moved: number;
  copied: number;
  skipped: number;
  decorated: number;
  subtitles: number;
  failures: string[];
  warnings: string[];
}

export interface SubtitleFetchReport {
  added: number;
  alreadyHad: number;
  failures: string[];
}

export interface ReapplyReport {
  updated: number;
  failures: string[];
}

export interface SeasonInfo {
  season: number;
  episodes: number;
}

export interface CatalogFile {
  path: string;
  season: number | null;
  episode: number | null;
  episodeTitle: string | null;
  resolution: string | null;
  size: number;
}

/** A collection entry as rendered: persisted fields plus live on-disk facts. */
export interface CatalogEntry {
  key: string;
  tmdbId: number;
  kind: MediaKind;
  title: string;
  originalTitle: string;
  year: number | null;
  overview: string;
  tagline: string | null;
  posterPath: string | null;
  backdropPath: string | null;
  genres: string[];
  runtimeMinutes: number | null;
  voteAverage: number;
  imdbId: string | null;
  status: string | null;
  seasons: SeasonInfo[];
  folder: string;
  files: CatalogFile[];
  addedAt: string;
  updatedAt: string;
  watched: boolean;
  watchedAt: string | null;
  rating: number | null;
  note: string;
  episodesWatched: string[];
  onDisk: boolean;
  episodesHave: number;
  episodesTotal: number;
}

export interface CatalogPatch {
  watched?: boolean;
  /** null clears the rating. */
  rating?: number | null;
  note?: string;
  episodesWatched?: string[];
}

export interface ImportReport {
  added: number;
  files: number;
  unmatched: string[];
}

export interface RunSummary {
  filesMoved: number;
  filesCopied: number;
  foldersCreated: number;
  filesWritten: number;
  failures: number;
}

export type UndoOp =
  | { type: "moved"; from: string; to: string }
  | { type: "copied"; from: string; to: string }
  | { type: "createdDir"; path: string }
  | { type: "wroteFile"; path: string }
  | { type: "setAttributes"; path: string; previous: number };

export interface UndoRun {
  id: string;
  startedAt: string;
  finishedAt: string | null;
  sourceRoot: string;
  libraryRoot: string;
  summary: RunSummary;
  entries: UndoOp[];
  undoneAt: string | null;
}

export interface RollbackReport {
  restored: number;
  skipped: number;
  failures: string[];
}

export interface ProgressEvent {
  phase: "scan" | "identify" | "move" | "decorate" | "subtitles" | "done";
  current: number;
  total: number;
  label: string;
  itemId: string | null;
  error: string | null;
}

const PROGRESS_EVENT = "cinefold://progress";

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) =>
    invoke<Settings>("save_settings", { settings }),
  validateApiKey: (key: string) => invoke<boolean>("validate_api_key", { key }),
  validateSubdlKey: (key: string) =>
    invoke<boolean>("validate_subdl_key", { key }),

  scanFolder: (path: string) => invoke<ScanResult>("scan_folder", { path }),
  getItems: () => invoke<MediaItem[]>("get_items"),
  includeSkipped: (path: string) =>
    invoke<MediaItem>("include_skipped", { path }),

  searchTitles: (query: string, kind: MediaKind, year?: number | null) =>
    invoke<Candidate[]>("search_titles", { query, kind, year: year ?? null }),
  applyMatch: (itemId: string, candidate: Candidate) =>
    invoke<MediaItem>("apply_match", { itemId, candidate }),
  approveItems: (itemIds: string[]) =>
    invoke<MediaItem[]>("approve_items", { itemIds }),
  setItemStatus: (itemId: string, status: ItemStatus) =>
    invoke<MediaItem>("set_item_status", { itemId, status }),
  setItemEpisode: (
    itemId: string,
    season: number | null,
    episode: number | null,
  ) => invoke<MediaItem>("set_item_episode", { itemId, season, episode }),

  buildPlan: () => invoke<Plan>("build_plan"),
  executePlan: (plan: Plan) => invoke<ExecSummary>("execute_plan", { plan }),

  catalogList: () => invoke<CatalogEntry[]>("catalog_list"),
  catalogUpdate: (key: string, patch: CatalogPatch) =>
    invoke<CatalogEntry>("catalog_update", { key, patch }),
  catalogRemove: (key: string) => invoke<void>("catalog_remove", { key }),
  catalogFetchSubtitles: (key: string) =>
    invoke<SubtitleFetchReport>("catalog_fetch_subtitles", { key }),
  catalogImportLibrary: () =>
    invoke<ImportReport>("catalog_import_library"),

  listUndoRuns: () => invoke<UndoRun[]>("list_undo_runs"),
  undoRun: (runId: string) => invoke<RollbackReport>("undo_run", { runId }),
  refreshIconCache: () => invoke<void>("refresh_icon_cache"),
  reapplyLibraryIcons: () => invoke<ReapplyReport>("reapply_library_icons"),
  /** Returns PNG data URLs at 128, 48 and 16 px. */
  previewIcon: (
    settings: Settings,
    posterUrl: string | null,
    kind: MediaKind = "movie",
  ) => invoke<string[]>("preview_icon", { settings, posterUrl, kind }),
  appPaths: () => invoke<Record<string, string>>("app_paths"),

  onProgress: (handler: (event: ProgressEvent) => void): Promise<UnlistenFn> =>
    listen<ProgressEvent>(PROGRESS_EVENT, (event) => handler(event.payload)),
};

// --- small formatting helpers shared by the views --------------------------

export function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const exponent = Math.min(
    Math.floor(Math.log(bytes) / Math.log(1024)),
    units.length - 1,
  );
  const value = bytes / Math.pow(1024, exponent);
  return `${value >= 10 || exponent === 0 ? Math.round(value) : value.toFixed(1)} ${units[exponent]}`;
}

export function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? path;
}

/** Path relative to a root, for compact destination display. */
export function relativeTo(path: string, root: string): string {
  if (!root) return path;
  const normalise = (value: string) => value.replace(/[\\/]+$/, "").toLowerCase();
  if (normalise(path).startsWith(normalise(root))) {
    return path.slice(root.replace(/[\\/]+$/, "").length + 1);
  }
  return path;
}

export function episodeLabel(item: MediaItem): string | null {
  const { season, episode, extraEpisodes } = item.parsed;
  if (season === null && episode === null) return null;
  const s = season !== null ? `S${String(season).padStart(2, "0")}` : "";
  if (episode === null) return s || null;
  const extras = extraEpisodes
    .map((n) => `-E${String(n).padStart(2, "0")}`)
    .join("");
  return `${s}E${String(episode).padStart(2, "0")}${extras}`;
}
