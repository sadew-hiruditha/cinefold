import { useEffect, useState } from "react";
import {
  api,
  fileName,
  type Candidate,
  type MediaItem,
  type MediaKind,
} from "../lib/api";
import { Badge, Button, Modal, Spinner, inputClass } from "./ui";

const POSTER_ROOT = "https://image.tmdb.org/t/p/w92";

function CandidateRow({
  candidate,
  selected,
  onPick,
}: {
  candidate: Candidate;
  selected: boolean;
  onPick: () => void;
}) {
  return (
    <button
      onClick={onPick}
      className={`flex w-full items-start gap-3 rounded-lg p-2.5 text-left transition-colors ${
        selected ? "bg-indigo-500/15 ring-1 ring-indigo-400/40" : "hover:bg-white/5"
      }`}
    >
      {candidate.posterPath ? (
        <img
          src={`${POSTER_ROOT}${candidate.posterPath}`}
          alt=""
          loading="lazy"
          className="h-[72px] w-[48px] shrink-0 rounded object-cover ring-1 ring-white/10"
        />
      ) : (
        <div className="grid h-[72px] w-[48px] shrink-0 place-items-center rounded bg-white/5 text-[10px] text-zinc-600">
          No art
        </div>
      )}
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium text-zinc-100">
            {candidate.title}
          </span>
          {candidate.year && (
            <span className="text-sm text-zinc-500">{candidate.year}</span>
          )}
          <Badge tone={candidate.score >= 0.85 ? "good" : "neutral"}>
            {Math.round(candidate.score * 100)}%
          </Badge>
        </div>
        {candidate.originalTitle !== candidate.title && (
          <div className="truncate text-xs text-zinc-500">
            {candidate.originalTitle}
          </div>
        )}
        <p className="mt-1 line-clamp-2 text-xs text-zinc-400">
          {candidate.overview || "No description."}
        </p>
      </div>
    </button>
  );
}

export function MatchDialog({
  item,
  onApply,
  onEditEpisode,
  onClose,
}: {
  item: MediaItem;
  onApply: (candidate: Candidate) => Promise<void>;
  onEditEpisode: (season: number | null, episode: number | null) => Promise<void>;
  onClose: () => void;
}) {
  const [kind, setKind] = useState<MediaKind>(item.parsed.kind);
  const [query, setQuery] = useState(item.parsed.title);
  const [year, setYear] = useState(item.parsed.year?.toString() ?? "");
  const [season, setSeason] = useState(item.parsed.season?.toString() ?? "");
  const [episode, setEpisode] = useState(item.parsed.episode?.toString() ?? "");
  const [results, setResults] = useState<Candidate[]>([]);
  const [searching, setSearching] = useState(false);
  const [applying, setApplying] = useState(false);
  const [selected, setSelected] = useState<Candidate | null>(
    item.identification.best,
  );
  const [error, setError] = useState<string | null>(null);

  // Start from what the lookup already found rather than an empty list.
  useEffect(() => {
    const seed = [
      item.identification.best,
      ...item.identification.alternatives,
    ].filter((c): c is Candidate => c !== null);
    setResults(seed);
  }, [item]);

  async function search() {
    setSearching(true);
    setError(null);
    try {
      const parsedYear = year.trim() === "" ? null : Number(year);
      const found = await api.searchTitles(
        query,
        kind,
        Number.isFinite(parsedYear) ? parsedYear : null,
      );
      setResults(found);
      if (found.length === 0) setError("No results for that search.");
    } catch (err) {
      setError(String(err));
    } finally {
      setSearching(false);
    }
  }

  async function confirm() {
    if (!selected) return;
    setApplying(true);
    setError(null);
    try {
      // Persist any episode correction first so the name is built from it.
      if (kind === "tv") {
        await onEditEpisode(
          season.trim() === "" ? null : Number(season),
          episode.trim() === "" ? null : Number(episode),
        );
      }
      await onApply(selected);
      onClose();
    } catch (err) {
      setError(String(err));
      setApplying(false);
    }
  }

  return (
    <Modal
      title="Which film or series is this?"
      subtitle={fileName(item.source)}
      onClose={onClose}
      wide
      footer={
        <>
          <span className="mr-auto text-xs text-zinc-500">
            The {item.parsed.titleSource === "folder" ? "folder name" : "filename"}{" "}
            suggests &ldquo;{item.parsed.title || "nothing"}&rdquo;
            {item.parsed.year ? ` (${item.parsed.year})` : ""}
          </span>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={confirm}
            disabled={!selected || applying}
          >
            {applying && <Spinner />}
            That&rsquo;s the one
          </Button>
        </>
      }
    >
      <div className="space-y-4">
        <div className="flex flex-wrap items-end gap-2">
          <div className="flex overflow-hidden rounded-lg ring-1 ring-white/10">
            {(["movie", "tv"] as MediaKind[]).map((option) => (
              <button
                key={option}
                onClick={() => setKind(option)}
                className={`px-3 py-2 text-sm transition-colors ${
                  kind === option
                    ? "bg-indigo-500 text-white"
                    : "bg-zinc-950/60 text-zinc-400 hover:text-zinc-200"
                }`}
              >
                {option === "movie" ? "Film" : "Series"}
              </button>
            ))}
          </div>

          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => event.key === "Enter" && search()}
            placeholder="Search by title"
            className={`${inputClass} mt-0 min-w-0 flex-1`}
          />
          <input
            value={year}
            onChange={(event) => setYear(event.target.value)}
            onKeyDown={(event) => event.key === "Enter" && search()}
            placeholder="Year"
            inputMode="numeric"
            className={`${inputClass} mt-0 w-24`}
          />
          <Button onClick={search} disabled={searching || query.trim() === ""}>
            {searching && <Spinner />}
            Search
          </Button>
        </div>

        {kind === "tv" && (
          <div className="flex items-center gap-3 rounded-lg bg-white/[0.03] px-3 py-2.5">
            <span className="text-sm text-zinc-400">Episode</span>
            <label className="flex items-center gap-1.5 text-sm text-zinc-400">
              S
              <input
                value={season}
                onChange={(event) => setSeason(event.target.value)}
                inputMode="numeric"
                className={`${inputClass} mt-0 w-16`}
              />
            </label>
            <label className="flex items-center gap-1.5 text-sm text-zinc-400">
              E
              <input
                value={episode}
                onChange={(event) => setEpisode(event.target.value)}
                inputMode="numeric"
                className={`${inputClass} mt-0 w-16`}
              />
            </label>
            <span className="text-xs text-zinc-500">
              Fix these if the episode number was read wrongly.
            </span>
          </div>
        )}

        {error && (
          <p className="rounded-lg bg-red-500/10 px-3 py-2 text-sm text-red-300">
            {error}
          </p>
        )}

        <div className="space-y-1">
          {results.length === 0 && !searching ? (
            <p className="px-1 py-8 text-center text-sm text-zinc-500">
              Type the title above to search.
            </p>
          ) : (
            results.map((candidate) => (
              <CandidateRow
                key={`${candidate.kind}-${candidate.id}`}
                candidate={candidate}
                selected={
                  selected?.id === candidate.id && selected?.kind === candidate.kind
                }
                onPick={() => setSelected(candidate)}
              />
            ))
          )}
        </div>
      </div>
    </Modal>
  );
}
