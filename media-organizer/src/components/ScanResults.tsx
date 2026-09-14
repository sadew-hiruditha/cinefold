import { useMemo, useState } from "react";
import {
  episodeLabel,
  fileName,
  formatBytes,
  type MediaItem,
  type SkippedFile,
} from "../lib/api";
import { Badge, Button, EmptyState, Panel, Spinner, SubtitleIcon } from "./ui";

type Tab = "ready" | "review" | "excluded" | "skipped";

const POSTER_ROOT = "https://image.tmdb.org/t/p/w92";

function confidenceTone(confidence: number) {
  if (confidence >= 0.85) return "good" as const;
  if (confidence >= 0.7) return "warn" as const;
  return "bad" as const;
}

function Poster({ path, title }: { path: string | null; title: string }) {
  if (!path) {
    return (
      <div className="grid h-[68px] w-[46px] shrink-0 place-items-center rounded bg-white/5 text-[10px] text-zinc-600">
        No art
      </div>
    );
  }
  return (
    <img
      src={`${POSTER_ROOT}${path}`}
      alt={`Poster for ${title}`}
      loading="lazy"
      className="h-[68px] w-[46px] shrink-0 rounded object-cover ring-1 ring-white/10"
    />
  );
}

function ItemRow({
  item,
  onChangeMatch,
  onToggle,
  onApprove,
  onFetchSubtitle,
  fetchingSubtitle,
}: {
  item: MediaItem;
  onChangeMatch: (item: MediaItem) => void;
  onToggle: (item: MediaItem) => void;
  onApprove: (item: MediaItem) => void;
  onFetchSubtitle: (item: MediaItem) => void;
  fetchingSubtitle: boolean;
}) {
  const match = item.identification.best;
  const episode = episodeLabel(item);
  const excluded = item.status === "excluded";
  const canApprove = item.status === "needsReview" && match !== null;

  return (
    <div
      className={`flex items-start gap-3 px-4 py-3 transition-colors hover:bg-white/[0.03] ${excluded ? "opacity-45" : ""}`}
    >
      <Poster path={match?.posterPath ?? null} title={item.parsed.title} />

      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span className="truncate text-sm font-medium text-zinc-100">
            {match?.title ?? (item.parsed.title || "Unrecognised")}
          </span>
          {match?.year && (
            <span className="text-sm text-zinc-500">{match.year}</span>
          )}
          {episode && <Badge tone="info">{episode}</Badge>}
          <Badge tone={item.parsed.kind === "tv" ? "info" : "neutral"}>
            {item.parsed.kind === "tv" ? "Series" : "Film"}
          </Badge>
          {item.identification.episodeTitle && (
            <span className="truncate text-xs text-zinc-400">
              {item.identification.episodeTitle}
            </span>
          )}
        </div>

        <div
          className="mt-1 truncate text-xs text-zinc-500"
          title={item.source}
        >
          {fileName(item.source)}
        </div>

        <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
          <Badge tone="neutral">{formatBytes(item.size)}</Badge>
          {item.parsed.resolution && (
            <Badge tone="neutral">{item.parsed.resolution}</Badge>
          )}
          {item.parsed.source && <Badge tone="neutral">{item.parsed.source}</Badge>}
          {item.subtitles.length > 0 ? (
            <Badge tone="good">
              {item.subtitles.length} subtitle
              {item.subtitles.length > 1 ? "s" : ""}
              {item.subtitles[0].language ? ` · ${item.subtitles
                .map((s) => s.language ?? "?")
                .join(", ")}` : ""}
            </Badge>
          ) : (
            <button
              onClick={() => onFetchSubtitle(item)}
              disabled={fetchingSubtitle}
              title="Search SubDL for a subtitle in your preferred language"
              className="inline-flex shrink-0 items-center gap-1 rounded-md bg-indigo-500/15 px-2 py-0.5 text-[11px] font-medium text-indigo-300 ring-1 ring-inset ring-indigo-400/30 transition-colors hover:bg-indigo-500/25 hover:text-indigo-200 disabled:opacity-50"
            >
              {fetchingSubtitle ? (
                <Spinner className="h-3 w-3" />
              ) : (
                <SubtitleIcon className="h-3 w-3" />
              )}
              {fetchingSubtitle ? "Fetching subtitle…" : "Get subtitle"}
            </button>
          )}
          {match && (
            <Badge tone={confidenceTone(item.identification.confidence)}>
              {Math.round(item.identification.confidence * 100)}% match
            </Badge>
          )}
          {item.identification.reason && item.status !== "ready" && (
            <span className="text-xs text-amber-300/80">
              {item.identification.reason}
            </span>
          )}
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-1.5">
        {canApprove && (
          <Button
            variant="primary"
            onClick={() => onApprove(item)}
            title="Yes, this is the right title"
          >
            Confirm
          </Button>
        )}
        <Button variant="ghost" onClick={() => onChangeMatch(item)}>
          {match ? "Wrong title?" : "Find title"}
        </Button>
        <Button
          variant="ghost"
          onClick={() => onToggle(item)}
          title={excluded ? "Put it back in" : "Leave this one alone"}
        >
          {excluded ? "Put back" : "Leave out"}
        </Button>
      </div>
    </div>
  );
}

export function ScanResults({
  items,
  skipped,
  onChangeMatch,
  onToggle,
  onInclude,
  including,
  onApprove,
  onExcludeAll,
  bulkBusy,
  onFetchSubtitle,
  fetchingSubtitleId,
}: {
  items: MediaItem[];
  skipped: SkippedFile[];
  onChangeMatch: (item: MediaItem) => void;
  onToggle: (item: MediaItem) => void;
  onInclude: (file: SkippedFile) => void;
  /** Path of the skipped file currently being identified, if any. */
  including: string | null;
  onApprove: (items: MediaItem[]) => void;
  onExcludeAll: (items: MediaItem[]) => void;
  bulkBusy: boolean;
  onFetchSubtitle: (item: MediaItem) => void;
  /** id of the item currently fetching a subtitle, if any. */
  fetchingSubtitleId: string | null;
}) {
  const buckets = useMemo(
    () => ({
      ready: items.filter((i) => i.status === "ready"),
      review: items.filter((i) => i.status === "needsReview"),
      excluded: items.filter((i) => i.status === "excluded"),
    }),
    [items],
  );

  // Land on whatever needs attention first.
  const [tab, setTab] = useState<Tab>(
    buckets.review.length > 0 ? "review" : "ready",
  );

  const tabs: { id: Tab; label: string; count: number }[] = [
    { id: "ready", label: "Ready", count: buckets.ready.length },
    { id: "review", label: "Needs a look", count: buckets.review.length },
    { id: "excluded", label: "Left out", count: buckets.excluded.length },
    { id: "skipped", label: "Skipped", count: skipped.length },
  ];

  const visible = tab === "skipped" ? [] : buckets[tab];

  return (
    <Panel className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="flex shrink-0 items-center gap-1 border-b border-white/5 px-3 py-2">
        {tabs.map((entry) => (
          <button
            key={entry.id}
            onClick={() => setTab(entry.id)}
            className={`rounded-lg px-3 py-1.5 text-sm transition-colors ${
              tab === entry.id
                ? "bg-white/10 text-white"
                : "text-zinc-400 hover:bg-white/5 hover:text-zinc-200"
            }`}
          >
            {entry.label}
            <span className="ml-1.5 text-xs text-zinc-500">{entry.count}</span>
          </button>
        ))}

        {tab === "review" && buckets.review.length > 0 && (
          <div className="ml-auto flex items-center gap-1.5">
            <Button
              variant="ghost"
              onClick={() => onExcludeAll(buckets.review)}
              disabled={bulkBusy}
              title="Leave every title in this list alone"
            >
              Leave all out
            </Button>
            <Button
              variant="primary"
              onClick={() =>
                onApprove(buckets.review.filter((i) => i.identification.best))
              }
              disabled={
                bulkBusy || !buckets.review.some((i) => i.identification.best)
              }
              title="Accept the suggested title for everything in this list"
            >
              {bulkBusy && <Spinner />}
              Confirm all
            </Button>
          </div>
        )}
      </div>

      <div className="min-h-0 flex-1 divide-y divide-white/5 overflow-y-auto">
        {tab === "skipped" ? (
          skipped.length === 0 ? (
            <EmptyState title="Nothing was skipped" />
          ) : (
            skipped.map((file) => (
              <div
                key={file.path}
                className="flex items-center justify-between gap-4 px-4 py-2.5"
              >
                <span className="truncate text-sm text-zinc-400" title={file.path}>
                  {fileName(file.path)}
                </span>
                <div className="flex shrink-0 items-center gap-2">
                  <Badge tone="neutral">{formatBytes(file.size)}</Badge>
                  <Badge tone="warn">{file.reason}</Badge>
                  <Button
                    variant="ghost"
                    onClick={() => onInclude(file)}
                    disabled={including !== null}
                    title="Treat this as a film or episode after all"
                  >
                    {including === file.path && <Spinner />}
                    Organise anyway
                  </Button>
                </div>
              </div>
            ))
          )
        ) : visible.length === 0 ? (
          <EmptyState
            title={
              tab === "review"
                ? "Every title was recognised"
                : tab === "excluded"
                  ? "Nothing has been left out"
                  : "No titles ready yet"
            }
          >
            {tab === "ready" &&
              "Confirm the titles under Needs a look and they will appear here."}
          </EmptyState>
        ) : (
          visible.map((item) => (
            <ItemRow
              key={item.id}
              item={item}
              onChangeMatch={onChangeMatch}
              onToggle={onToggle}
              onApprove={(single) => onApprove([single])}
              onFetchSubtitle={onFetchSubtitle}
              fetchingSubtitle={fetchingSubtitleId === item.id}
            />
          ))
        )}
      </div>
    </Panel>
  );
}
