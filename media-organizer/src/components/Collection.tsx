import { useCallback, useEffect, useMemo, useState } from "react";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  api,
  formatBytes,
  type CatalogEntry,
  type ImportReport,
} from "../lib/api";
import {
  Badge,
  Button,
  CloseIcon,
  EmptyState,
  Panel,
  Select,
  Spinner,
  inputClass,
} from "./ui";

const POSTER = "https://image.tmdb.org/t/p/w342";
const BACKDROP = "https://image.tmdb.org/t/p/w780";

type Filter = "all" | "movie" | "tv" | "unwatched";
type Sort = "added" | "title" | "year" | "rating";

function runtimeLabel(minutes: number | null): string | null {
  if (!minutes) return null;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return h > 0 ? `${h}h ${m > 0 ? `${m}m` : ""}`.trim() : `${m}m`;
}

function episodeKey(season: number, episode: number): string {
  return `S${String(season).padStart(2, "0")}E${String(episode).padStart(2, "0")}`;
}

function Stars({
  value,
  onChange,
}: {
  value: number | null;
  onChange: (rating: number | null) => void;
}) {
  const [hover, setHover] = useState<number | null>(null);
  const shown = hover ?? value ?? 0;
  return (
    <div className="flex items-center gap-1" onMouseLeave={() => setHover(null)}>
      {Array.from({ length: 10 }, (_, i) => i + 1).map((n) => (
        <button
          key={n}
          type="button"
          onMouseEnter={() => setHover(n)}
          onClick={() => onChange(value === n ? null : n)}
          title={`${n} / 10`}
          className={`h-5 w-3 rounded-sm transition-colors ${
            n <= shown ? "bg-amber-400" : "bg-white/10 hover:bg-white/20"
          }`}
        />
      ))}
      <span className="ml-2 w-10 text-xs tabular-nums text-zinc-400">
        {value ? `${value}/10` : "—"}
      </span>
    </div>
  );
}

function Card({
  entry,
  selected,
  onSelect,
}: {
  entry: CatalogEntry;
  selected: boolean;
  onSelect: () => void;
}) {
  const progress =
    entry.kind === "tv" && entry.episodesTotal > 0
      ? `${entry.episodesHave}/${entry.episodesTotal}`
      : null;

  return (
    <button
      onClick={onSelect}
      className={`group relative flex flex-col overflow-hidden rounded-xl bg-zinc-900 text-left ring-1 transition-all hover:-translate-y-0.5 ${
        selected ? "ring-indigo-400" : "ring-white/5 hover:ring-white/20"
      }`}
    >
      <div className="relative aspect-[2/3] w-full bg-white/5">
        {entry.posterPath ? (
          <img
            src={`${POSTER}${entry.posterPath}`}
            alt=""
            loading="lazy"
            className={`h-full w-full object-cover ${entry.watched ? "opacity-60" : ""}`}
          />
        ) : (
          <div className="grid h-full place-items-center text-xs text-zinc-600">
            No art
          </div>
        )}
        {entry.watched && (
          <div className="absolute left-1.5 top-1.5">
            <Badge tone="good">watched</Badge>
          </div>
        )}
        {(progress || entry.rating) && (
          <div className="absolute bottom-1.5 right-1.5 flex gap-1">
            {progress && <Badge tone="info">{progress}</Badge>}
            {entry.rating && <Badge tone="neutral">★ {entry.rating}</Badge>}
          </div>
        )}
      </div>
      <div className="px-2.5 py-2">
        <p className="truncate text-sm font-medium text-zinc-100">{entry.title}</p>
        <p className="text-xs text-zinc-500">
          {entry.year ?? "—"} · {entry.kind === "tv" ? "Series" : "Film"}
        </p>
      </div>
    </button>
  );
}

function Detail({
  entry,
  onChange,
  onRemove,
  onClose,
}: {
  entry: CatalogEntry;
  onChange: (updated: CatalogEntry) => void;
  onRemove: () => void;
  onClose: () => void;
}) {
  const [note, setNote] = useState(entry.note);
  const [saving, setSaving] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [fetching, setFetching] = useState(false);
  const [fetchNote, setFetchNote] = useState<string | null>(null);

  async function fetchSubtitles() {
    setFetching(true);
    setFetchNote(null);
    try {
      const report = await api.catalogFetchSubtitles(entry.key);
      setFetchNote(
        report.added > 0
          ? `Added ${report.added} subtitle${report.added === 1 ? "" : "s"}.`
          : report.failures[0] ?? "Every file already has a subtitle.",
      );
    } catch (err) {
      setFetchNote(String(err));
    } finally {
      setFetching(false);
    }
  }

  useEffect(() => {
    setNote(entry.note);
    setConfirmRemove(false);
    setFetchNote(null);
  }, [entry.key, entry.note]);

  async function patch(fields: Parameters<typeof api.catalogUpdate>[1]) {
    setSaving(true);
    try {
      onChange(await api.catalogUpdate(entry.key, fields));
    } finally {
      setSaving(false);
    }
  }

  const watchedSet = useMemo(() => new Set(entry.episodesWatched), [entry]);

  function toggleEpisode(key: string) {
    const next = new Set(watchedSet);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    patch({ episodesWatched: [...next] });
  }

  const totalSize = entry.files.reduce((sum, f) => sum + f.size, 0);
  const runtime = runtimeLabel(entry.runtimeMinutes);

  return (
    <aside className="flex h-full w-[380px] shrink-0 flex-col overflow-hidden rounded-xl bg-zinc-900 ring-1 ring-white/5">
      <div className="relative h-40 shrink-0 bg-zinc-950">
        {entry.backdropPath && (
          <img
            src={`${BACKDROP}${entry.backdropPath}`}
            alt=""
            className="h-full w-full object-cover opacity-60"
          />
        )}
        <div className="absolute inset-0 bg-linear-to-t from-zinc-900 to-transparent" />
        <button
          onClick={onClose}
          className="absolute right-2 top-2 rounded-lg bg-black/40 p-1.5 text-zinc-300 hover:bg-black/60 hover:text-white"
          title="Close"
        >
          <CloseIcon />
        </button>
        <div className="absolute bottom-3 left-4 right-4">
          <h2 className="text-lg font-semibold leading-tight text-white">
            {entry.title}
          </h2>
          <p className="text-sm text-zinc-300">
            {entry.year ?? ""}
            {runtime ? ` · ${runtime}` : ""}
            {entry.voteAverage > 0 ? ` · TMDB ${entry.voteAverage.toFixed(1)}` : ""}
          </p>
        </div>
      </div>

      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-4 py-4">
        {entry.tagline && (
          <p className="text-sm italic text-zinc-400">{entry.tagline}</p>
        )}

        {entry.genres.length > 0 && (
          <div className="flex flex-wrap gap-1">
            {entry.genres.map((g) => (
              <Badge key={g} tone="neutral">
                {g}
              </Badge>
            ))}
          </div>
        )}

        <p className="text-sm leading-relaxed text-zinc-300">{entry.overview}</p>

        <div className="space-y-2 rounded-lg bg-white/[0.03] p-3">
          <label className="flex cursor-pointer items-center justify-between text-sm">
            <span className="text-zinc-200">Watched</span>
            <input
              type="checkbox"
              checked={entry.watched}
              onChange={(e) => patch({ watched: e.target.checked })}
              className="h-4 w-4 accent-indigo-500"
            />
          </label>
          <div className="flex items-center justify-between text-sm">
            <span className="text-zinc-200">Your rating</span>
            <Stars value={entry.rating} onChange={(rating) => patch({ rating })} />
          </div>
          <textarea
            value={note}
            onChange={(e) => setNote(e.target.value)}
            onBlur={() => note !== entry.note && patch({ note })}
            placeholder="A note to yourself"
            rows={2}
            className={`${inputClass} resize-none`}
          />
        </div>

        {entry.kind === "tv" && entry.seasons.length > 0 && (
          <div>
            <h3 className="mb-1.5 text-xs font-semibold uppercase tracking-wide text-zinc-500">
              Episodes · {entry.episodesHave} of {entry.episodesTotal} on disk
            </h3>
            <div className="space-y-2">
              {entry.seasons.map((season) => (
                <div key={season.season}>
                  <p className="mb-1 text-xs text-zinc-400">
                    Season {season.season}
                  </p>
                  <div className="flex flex-wrap gap-1">
                    {Array.from({ length: season.episodes }, (_, i) => i + 1).map(
                      (ep) => {
                        const key = episodeKey(season.season, ep);
                        const have = entry.files.some(
                          (f) => f.season === season.season && f.episode === ep,
                        );
                        const seen = watchedSet.has(key);
                        return (
                          <button
                            key={key}
                            onClick={() => toggleEpisode(key)}
                            title={`${key}${have ? "" : " · not on disk"}${seen ? " · watched" : ""}`}
                            className={`h-6 w-7 rounded text-[10px] tabular-nums transition-colors ${
                              seen
                                ? "bg-emerald-500/30 text-emerald-200"
                                : have
                                  ? "bg-white/10 text-zinc-200 hover:bg-white/20"
                                  : "bg-transparent text-zinc-600 ring-1 ring-inset ring-white/10"
                            }`}
                          >
                            {ep}
                          </button>
                        );
                      },
                    )}
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        <div>
          <h3 className="mb-1.5 text-xs font-semibold uppercase tracking-wide text-zinc-500">
            On disk · {entry.files.length} file{entry.files.length === 1 ? "" : "s"} ·{" "}
            {formatBytes(totalSize)}
          </h3>
          <p
            className="truncate font-mono text-xs text-zinc-500"
            title={entry.folder}
          >
            {entry.folder}
          </p>
          <div className="mt-2 flex items-center gap-2">
            <Button onClick={fetchSubtitles} disabled={fetching || !entry.onDisk}>
              {fetching && <Spinner />}
              Get subtitles
            </Button>
            {fetchNote && <span className="text-xs text-zinc-400">{fetchNote}</span>}
          </div>
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-2 border-t border-white/5 px-4 py-3">
        <Button
          onClick={() => revealItemInDir(entry.folder)}
          disabled={!entry.onDisk}
        >
          Open folder
        </Button>
        <Button
          variant="ghost"
          onClick={() =>
            openUrl(
              `https://www.themoviedb.org/${entry.kind === "tv" ? "tv" : "movie"}/${entry.tmdbId}`,
            )
          }
        >
          TMDB
        </Button>
        <span className="ml-auto" />
        {confirmRemove ? (
          <>
            <Button variant="ghost" onClick={() => setConfirmRemove(false)}>
              Keep
            </Button>
            <Button variant="danger" onClick={onRemove}>
              Remove
            </Button>
          </>
        ) : (
          <Button
            variant="ghost"
            onClick={() => setConfirmRemove(true)}
            title="Take it off this list. The files stay where they are."
          >
            Remove
          </Button>
        )}
        {saving && <Spinner className="text-zinc-500" />}
      </div>
    </aside>
  );
}

export function CollectionView({ canImport }: { canImport: boolean }) {
  const [entries, setEntries] = useState<CatalogEntry[] | null>(null);
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [sort, setSort] = useState<Sort>("added");
  const [query, setQuery] = useState("");
  const [importing, setImporting] = useState(false);
  const [importReport, setImportReport] = useState<ImportReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setEntries(await api.catalogList());
    } catch (err) {
      setError(String(err));
      setEntries([]);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  async function importLibrary() {
    setImporting(true);
    setImportReport(null);
    setError(null);
    try {
      setImportReport(await api.catalogImportLibrary());
      await refresh();
    } catch (err) {
      setError(String(err));
    } finally {
      setImporting(false);
    }
  }

  function replace(updated: CatalogEntry) {
    setEntries((current) =>
      current ? current.map((e) => (e.key === updated.key ? updated : e)) : current,
    );
  }

  async function remove(key: string) {
    await api.catalogRemove(key);
    setEntries((current) => current?.filter((e) => e.key !== key) ?? null);
    setSelectedKey(null);
  }

  const visible = useMemo(() => {
    if (!entries) return [];
    const q = query.trim().toLowerCase();
    const filtered = entries.filter((e) => {
      if (filter === "movie" && e.kind !== "movie") return false;
      if (filter === "tv" && e.kind !== "tv") return false;
      if (filter === "unwatched" && e.watched) return false;
      if (q && !`${e.title} ${e.originalTitle} ${e.genres.join(" ")}`.toLowerCase().includes(q))
        return false;
      return true;
    });
    const sorted = [...filtered];
    switch (sort) {
      case "title":
        sorted.sort((a, b) => a.title.localeCompare(b.title));
        break;
      case "year":
        sorted.sort((a, b) => (b.year ?? 0) - (a.year ?? 0));
        break;
      case "rating":
        sorted.sort((a, b) => (b.rating ?? 0) - (a.rating ?? 0) || b.voteAverage - a.voteAverage);
        break;
      default:
        sorted.sort((a, b) => b.addedAt.localeCompare(a.addedAt));
    }
    return sorted;
  }, [entries, filter, sort, query]);

  const selected = entries?.find((e) => e.key === selectedKey) ?? null;

  const filters: { id: Filter; label: string }[] = [
    { id: "all", label: "All" },
    { id: "movie", label: "Films" },
    { id: "tv", label: "Series" },
    { id: "unwatched", label: "Unwatched" },
  ];

  if (entries === null) {
    return (
      <div className="flex items-center gap-2 p-6 text-sm text-zinc-400">
        <Spinner /> Loading collection
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 gap-4">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3">
        <Panel className="flex flex-wrap items-center gap-2 p-2.5">
          {filters.map((f) => (
            <button
              key={f.id}
              onClick={() => setFilter(f.id)}
              className={`rounded-lg px-3 py-1.5 text-sm transition-colors ${
                filter === f.id
                  ? "bg-white/10 text-white"
                  : "text-zinc-400 hover:bg-white/5 hover:text-zinc-200"
              }`}
            >
              {f.label}
            </button>
          ))}
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search"
            className={`${inputClass} mt-0 ml-auto w-48`}
          />
          <Select<Sort>
            value={sort}
            onChange={setSort}
            className="w-40"
            options={[
              { value: "added", label: "Recently added" },
              { value: "title", label: "Title" },
              { value: "year", label: "Year" },
              { value: "rating", label: "Your rating" },
            ]}
          />
          <Button
            onClick={importLibrary}
            disabled={importing || !canImport}
            title={
              canImport
                ? "Add everything already in your library folder"
                : "Set a library folder and API key in Settings first"
            }
          >
            {importing && <Spinner />}
            Import from library
          </Button>
        </Panel>

        {error && (
          <p className="rounded-lg bg-red-500/10 px-4 py-2 text-sm text-red-300">{error}</p>
        )}
        {importReport && (
          <p className="rounded-lg bg-emerald-500/10 px-4 py-2 text-sm text-emerald-200">
            Added {importReport.added} title{importReport.added === 1 ? "" : "s"} (
            {importReport.files} files).
            {importReport.unmatched.length > 0 &&
              ` Could not identify: ${importReport.unmatched.slice(0, 5).join(", ")}${importReport.unmatched.length > 5 ? "…" : ""}`}
          </p>
        )}

        <div className="min-h-0 flex-1 overflow-y-auto">
          {entries.length === 0 ? (
            <Panel>
              <EmptyState title="No films or series yet">
                Every title Cinefold files into your library shows up here with
                its poster. Already have an organised library? Use{" "}
                <strong>Import from library</strong>.
              </EmptyState>
            </Panel>
          ) : visible.length === 0 ? (
            <Panel>
              <EmptyState title="Nothing matches that filter" />
            </Panel>
          ) : (
            <div className="grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-3 pb-4 pr-1">
              {visible.map((entry) => (
                <Card
                  key={entry.key}
                  entry={entry}
                  selected={entry.key === selectedKey}
                  onSelect={() =>
                    setSelectedKey(entry.key === selectedKey ? null : entry.key)
                  }
                />
              ))}
            </div>
          )}
        </div>

        <p className="shrink-0 text-xs text-zinc-500">
          {entries.length} title{entries.length === 1 ? "" : "s"} ·{" "}
          {entries.filter((e) => e.watched).length} watched
        </p>
      </div>

      {selected && (
        <Detail
          entry={selected}
          onChange={replace}
          onRemove={() => remove(selected.key)}
          onClose={() => setSelectedKey(null)}
        />
      )}
    </div>
  );
}
