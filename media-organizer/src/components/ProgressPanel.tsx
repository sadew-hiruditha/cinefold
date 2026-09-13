import type { ExecSummary, ProgressEvent } from "../lib/api";
import { Button, Modal, Spinner } from "./ui";

const PHASE_LABELS: Record<ProgressEvent["phase"], string> = {
  scan: "Looking for films and episodes",
  identify: "Recognising titles",
  move: "Filing into your library",
  decorate: "Putting posters on folders",
  subtitles: "Fetching missing subtitles",
  done: "Finished",
};

/** Live progress while a run executes, then the outcome. */
export function ProgressPanel({
  progress,
  errors,
  summary,
  onClose,
}: {
  progress: ProgressEvent | null;
  errors: string[];
  summary: ExecSummary | null;
  onClose: () => void;
}) {
  const running = summary === null;
  const percent =
    progress && progress.total > 0
      ? Math.round((progress.current / progress.total) * 100)
      : null;

  return (
    <Modal
      title={running ? "Building your library" : "Library updated"}
      onClose={() => {
        if (!running) onClose();
      }}
      footer={
        !running ? (
          <Button variant="primary" onClick={onClose}>
            Close
          </Button>
        ) : undefined
      }
    >
      {running && (
        <div className="space-y-3">
          <div className="flex items-center gap-2 text-sm text-zinc-200">
            <Spinner />
            {progress ? PHASE_LABELS[progress.phase] : "Starting"}
            {progress && progress.total > 0 && (
              <span className="text-zinc-500">
                {progress.current} / {progress.total}
              </span>
            )}
          </div>
          <div className="h-2 overflow-hidden rounded-full bg-white/10">
            <div
              className={`h-full rounded-full bg-indigo-500 transition-[width] duration-300 ${percent === null ? "w-1/3 animate-pulse" : ""}`}
              style={percent !== null ? { width: `${percent}%` } : undefined}
            />
          </div>
          {progress?.label && (
            <p className="truncate text-xs text-zinc-500" title={progress.label}>
              {progress.label}
            </p>
          )}
        </div>
      )}

      {summary && (
        <div className="space-y-3">
          <dl className="grid grid-cols-2 gap-2 sm:grid-cols-4">
            {[
              ["Filed", summary.moved + summary.copied],
              ["Posters added", summary.decorated],
              ["Subtitles fetched", summary.subtitles],
              ["Skipped", summary.skipped],
            ].map(([label, value]) => (
              <div
                key={label}
                className="rounded-lg bg-white/[0.04] px-3 py-2"
              >
                <dt className="text-xs text-zinc-500">{label}</dt>
                <dd className="text-lg font-semibold text-zinc-100">{value}</dd>
              </div>
            ))}
          </dl>
          <p className="text-xs text-zinc-500">
            Changed your mind? This run can be undone from History.
            {summary.decorated > 0 &&
              " Windows can take a moment to show the new folder posters."}
          </p>
        </div>
      )}

      {(errors.length > 0 ||
        (summary && (summary.failures.length > 0 || summary.warnings.length > 0))) && (
        <div className="mt-4 space-y-1">
          {[...(summary?.failures ?? errors)].map((message) => (
            <p
              key={message}
              className="rounded-lg bg-red-500/10 px-3 py-1.5 text-xs text-red-300"
            >
              {message}
            </p>
          ))}
          {summary?.warnings.map((message) => (
            <p
              key={message}
              className="rounded-lg bg-amber-500/10 px-3 py-1.5 text-xs text-amber-200"
            >
              {message}
            </p>
          ))}
        </div>
      )}
    </Modal>
  );
}
