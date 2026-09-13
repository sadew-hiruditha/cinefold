import { useMemo } from "react";
import {
  fileName,
  formatBytes,
  relativeTo,
  type Plan,
  type PlanEntry,
} from "../lib/api";
import { Badge, Button, Modal } from "./ui";

function EntryCard({
  entry,
  libraryRoot,
  onToggle,
}: {
  entry: PlanEntry;
  libraryRoot: string;
  onToggle: () => void;
}) {
  return (
    <div
      className={`rounded-xl bg-white/[0.03] ring-1 ring-white/5 ${entry.enabled ? "" : "opacity-50"}`}
    >
      <div className="flex items-start gap-3 px-4 py-3">
        <button
          onClick={onToggle}
          title={entry.enabled ? "Leave this title out" : "Include this title"}
          className={`mt-0.5 grid h-5 w-5 shrink-0 place-items-center rounded border transition-colors ${
            entry.enabled
              ? "border-indigo-400 bg-indigo-500 text-white"
              : "border-white/20 bg-transparent"
          }`}
        >
          {entry.enabled && (
            <svg viewBox="0 0 20 20" className="h-3.5 w-3.5" fill="none">
              <path
                d="M5 10.5l3 3 7-7"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          )}
        </button>

        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-zinc-100">{entry.title}</span>
            <Badge tone={entry.kind === "tv" ? "info" : "neutral"}>
              {entry.kind === "tv" ? "Series" : "Film"}
            </Badge>
            {entry.duplicateOf && <Badge tone="warn">duplicate</Badge>}
          </div>
          <div
            className="mt-0.5 truncate font-mono text-xs text-zinc-500"
            title={entry.destinationDir}
          >
            {relativeTo(entry.destinationDir, libraryRoot)}
          </div>

          <ul className="mt-2 space-y-1">
            {entry.ops.map((op) => (
              <li
                key={op.source}
                className="grid grid-cols-[1fr_auto_1fr] items-center gap-2 text-xs"
              >
                <span
                  className="truncate text-zinc-400"
                  title={op.source}
                >
                  {fileName(op.source)}
                </span>
                <span className="text-zinc-600">&rarr;</span>
                <span
                  className={`truncate ${op.kind === "video" ? "text-zinc-200" : "text-zinc-400"}`}
                  title={op.destination}
                >
                  {fileName(op.destination)}
                  {op.note && (
                    <span className="ml-1.5 text-amber-300/80">({op.note})</span>
                  )}
                </span>
              </li>
            ))}
          </ul>

          {entry.warnings.length > 0 && (
            <ul className="mt-2 space-y-0.5">
              {entry.warnings.map((warning) => (
                <li key={warning} className="text-xs text-amber-300/80">
                  {warning}
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

export function PlanPreview({
  plan,
  copyMode,
  onChange,
  onExecute,
  onClose,
}: {
  plan: Plan;
  copyMode: boolean;
  onChange: (plan: Plan) => void;
  onExecute: () => void;
  onClose: () => void;
}) {
  const enabled = useMemo(
    () => plan.entries.filter((entry) => entry.enabled),
    [plan],
  );
  const fileCount = enabled.reduce((sum, entry) => sum + entry.ops.length, 0);
  const bytes = enabled.reduce(
    (sum, entry) =>
      sum + entry.ops.reduce((inner, op) => inner + op.size, 0),
    0,
  );

  function toggle(itemId: string) {
    onChange({
      ...plan,
      entries: plan.entries.map((entry) =>
        entry.itemId === itemId ? { ...entry, enabled: !entry.enabled } : entry,
      ),
    });
  }

  const verb = copyMode ? "Copy" : "File";

  return (
    <Modal
      title="Your library after this"
      subtitle="Nothing has moved yet. Each title below goes where it says once you confirm."
      onClose={onClose}
      wide
      footer={
        <>
          <span className="mr-auto text-xs text-zinc-500">
            {fileCount} file{fileCount === 1 ? "" : "s"} · {formatBytes(bytes)}
            {plan.crossVolume && " · moving between drives, so each file is copied and checked"}
          </span>
          <Button variant="ghost" onClick={onClose}>
            Back
          </Button>
          <Button
            variant="primary"
            onClick={onExecute}
            disabled={fileCount === 0}
          >
            {verb} {fileCount} {fileCount === 1 ? "file" : "files"} into the library
          </Button>
        </>
      }
    >
      {plan.warnings.length > 0 && (
        <div className="mb-3 space-y-1 rounded-lg bg-amber-500/10 px-3 py-2 text-sm text-amber-200">
          {plan.warnings.map((warning) => (
            <p key={warning}>{warning}</p>
          ))}
        </div>
      )}

      <div className="mb-3 truncate font-mono text-xs text-zinc-500">
        Library: {plan.libraryRoot}
      </div>

      <div className="space-y-2">
        {plan.entries.map((entry) => (
          <EntryCard
            key={entry.itemId}
            entry={entry}
            libraryRoot={plan.libraryRoot}
            onToggle={() => toggle(entry.itemId)}
          />
        ))}
      </div>
    </Modal>
  );
}
