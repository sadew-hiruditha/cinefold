import { useCallback, useEffect, useMemo, useState } from "react";
import {
  api,
  fileName,
  relativeTo,
  type RollbackReport,
  type UndoRun,
} from "../lib/api";
import { Badge, Button, EmptyState, Panel, Spinner } from "./ui";

function dayLabel(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const today = new Date();
  const yesterday = new Date();
  yesterday.setDate(today.getDate() - 1);
  const sameDay = (a: Date, b: Date) => a.toDateString() === b.toDateString();
  if (sameDay(date, today)) return "Today";
  if (sameDay(date, yesterday)) return "Yesterday";
  return date.toLocaleDateString(undefined, {
    weekday: "long",
    day: "numeric",
    month: "long",
  });
}

function timeLabel(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime())
    ? ""
    : date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

/**
 * The titles a run touched, read off where the files ended up:
 * Library/Movies/Inception (2010)/… -> "Inception (2010)".
 */
function titlesOf(run: UndoRun): string[] {
  const seen = new Set<string>();
  for (const op of run.entries) {
    if (op.type !== "moved" && op.type !== "copied") continue;
    const rel = relativeTo(op.to, run.libraryRoot);
    const parts = rel.split(/[\\/]/).filter(Boolean);
    // [Movies | TV Shows, Title, ...] - the title is the second segment.
    const title = parts.length >= 3 ? parts[1] : parts[0];
    if (title) seen.add(title);
  }
  return [...seen];
}

function RunRow({
  run,
  onUndo,
  busy,
}: {
  run: UndoRun;
  onUndo: () => Promise<void>;
  busy: boolean;
}) {
  const [confirming, setConfirming] = useState(false);
  const [report, setReport] = useState<RollbackReport | null>(null);
  const undone = run.undoneAt !== null;
  const titles = useMemo(() => titlesOf(run), [run]);
  const shown = titles.slice(0, 4);
  const more = titles.length - shown.length;
  const files = run.summary.filesMoved + run.summary.filesCopied;

  async function undo() {
    setConfirming(false);
    try {
      await onUndo();
    } catch (err) {
      setReport({ restored: 0, skipped: 0, failures: [String(err)] });
    }
  }

  return (
    <div
      className={`flex items-start gap-4 px-4 py-3 ${undone ? "opacity-50" : ""}`}
      title={`Run ${run.id}`}
    >
      <span className="w-16 shrink-0 pt-0.5 text-sm tabular-nums text-zinc-500">
        {timeLabel(run.startedAt)}
      </span>

      <div className="min-w-0 flex-1">
        <p className="truncate text-sm text-zinc-100">
          {shown.length > 0 ? (
            <>
              {shown.join(", ")}
              {more > 0 && <span className="text-zinc-500"> and {more} more</span>}
            </>
          ) : (
            <span className="text-zinc-400">No titles filed</span>
          )}
        </p>
        <p className="mt-0.5 text-xs text-zinc-500">
          {files} {files === 1 ? "file" : "files"}
          {run.summary.filesCopied > 0 && ` (${run.summary.filesCopied} copied)`}
          {" · "}
          <span title={run.sourceRoot}>{fileName(run.sourceRoot) || run.sourceRoot}</span>
          {" → "}
          <span title={run.libraryRoot}>{fileName(run.libraryRoot) || run.libraryRoot}</span>
          {run.summary.failures > 0 && (
            <>
              {" · "}
              <span className="text-red-300">{run.summary.failures} failed</span>
            </>
          )}
        </p>

        {report && (
          <div className="mt-2 space-y-1 text-xs">
            <p className="text-zinc-300">
              Put back {report.restored}
              {report.skipped > 0 && `, ${report.skipped} already gone`}
              {report.failures.length > 0 && `, ${report.failures.length} could not be restored`}
            </p>
            {report.failures.map((failure) => (
              <p key={failure} className="rounded bg-red-500/10 px-2 py-1 text-red-300">
                {failure}
              </p>
            ))}
          </div>
        )}
      </div>

      <div className="flex shrink-0 items-center gap-2">
        {undone ? (
          <Badge tone="neutral">Undone</Badge>
        ) : confirming ? (
          <>
            <span className="text-xs text-zinc-400">Put everything back?</span>
            <Button variant="ghost" onClick={() => setConfirming(false)}>
              Keep
            </Button>
            <Button variant="danger" onClick={undo}>
              Undo
            </Button>
          </>
        ) : (
          <Button variant="ghost" onClick={() => setConfirming(true)} disabled={busy}>
            {busy && <Spinner />}
            Undo
          </Button>
        )}
      </div>
    </div>
  );
}

export function UndoHistory() {
  const [runs, setRuns] = useState<UndoRun[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [showUndone, setShowUndone] = useState(false);

  const refresh = useCallback(async () => {
    setRuns(await api.listUndoRuns());
  }, []);

  useEffect(() => {
    refresh().catch(() => setRuns([]));
  }, [refresh]);

  async function undo(runId: string) {
    setBusy(runId);
    try {
      await api.undoRun(runId);
      await refresh();
    } finally {
      setBusy(null);
    }
  }

  const groups = useMemo(() => {
    if (!runs) return [];
    const visible = showUndone ? runs : runs.filter((r) => r.undoneAt === null);
    const byDay = new Map<string, UndoRun[]>();
    for (const run of visible) {
      const label = dayLabel(run.startedAt);
      byDay.set(label, [...(byDay.get(label) ?? []), run]);
    }
    return [...byDay.entries()];
  }, [runs, showUndone]);

  if (runs === null) {
    return (
      <div className="flex items-center gap-2 p-6 text-sm text-zinc-400">
        <Spinner /> Loading history
      </div>
    );
  }

  const undoneCount = runs.filter((r) => r.undoneAt !== null).length;

  if (runs.length === 0) {
    return (
      <Panel className="mx-auto w-full max-w-3xl">
        <EmptyState title="Nothing organised yet">
          Every time films or episodes are filed into your library, the run
          appears here and can be undone.
        </EmptyState>
      </Panel>
    );
  }

  return (
    <div className="mx-auto w-full max-w-3xl space-y-5">
      {undoneCount > 0 && (
        <div className="flex justify-end">
          <button
            onClick={() => setShowUndone((v) => !v)}
            className="text-xs text-zinc-500 hover:text-zinc-300"
          >
            {showUndone ? "Hide" : "Show"} {undoneCount} undone{" "}
            {undoneCount === 1 ? "run" : "runs"}
          </button>
        </div>
      )}

      {groups.length === 0 && (
        <Panel>
          <EmptyState title="Everything has been undone">
            Show the undone runs above to see them.
          </EmptyState>
        </Panel>
      )}

      {groups.map(([day, dayRuns]) => (
        <section key={day}>
          <h3 className="mb-2 px-1 text-xs font-semibold uppercase tracking-wide text-zinc-500">
            {day}
          </h3>
          <Panel className="divide-y divide-white/5">
            {dayRuns.map((run) => (
              <RunRow
                key={run.id}
                run={run}
                busy={busy === run.id}
                onUndo={() => undo(run.id)}
              />
            ))}
          </Panel>
        </section>
      ))}
    </div>
  );
}
