import { open } from "@tauri-apps/plugin-dialog";
import type { ProgressEvent } from "../lib/api";
import { Button, FolderIcon, Spinner } from "./ui";

/**
 * The first thing a user sees: one big card to pick a folder and scan it.
 * Once a scan exists the app swaps this for the compact bar at the top.
 */
export function SourceHero({
  value,
  onChange,
  onScan,
  scanning,
  progress,
}: {
  value: string;
  onChange: (path: string) => void;
  onScan: () => void;
  scanning: boolean;
  progress: ProgressEvent | null;
}) {
  async function pick() {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "Choose the folder to organise",
      defaultPath: value || undefined,
    });
    if (typeof selected === "string" && selected.length > 0) {
      onChange(selected);
    }
  }

  const progressLabel =
    progress?.phase === "identify"
      ? `Recognising title ${progress.current} of ${progress.total}`
      : progress?.label
        ? `Looking for films and episodes · ${progress.label}`
        : "Looking for films and episodes";

  return (
    <div className="flex flex-1 items-center justify-center">
      <div className="w-full max-w-xl rounded-2xl bg-zinc-900/70 p-8 text-center ring-1 ring-white/5">
        <div className="mx-auto grid h-14 w-14 place-items-center rounded-2xl bg-indigo-500/15 text-indigo-300">
          <FolderIcon className="h-7 w-7" />
        </div>

        <h2 className="mt-4 text-xl font-semibold text-white">
          Where are your films and series?
        </h2>
        <p className="mt-1 text-sm text-zinc-400">
          Point Cinefold at a downloads folder. It finds every film and episode
          inside, whatever the files are called.
        </p>

        {scanning ? (
          <div className="mt-6 flex items-center justify-center gap-2 text-sm text-zinc-200">
            <Spinner />
            <span className="truncate">{progressLabel}</span>
          </div>
        ) : value ? (
          <>
            <div
              className="mt-6 truncate rounded-lg bg-zinc-950/60 px-4 py-3 text-left text-sm text-zinc-200 ring-1 ring-white/10"
              title={value}
            >
              {value}
            </div>
            <div className="mt-3 flex items-center justify-center gap-2">
              <Button onClick={pick}>Change folder</Button>
              <Button variant="primary" onClick={onScan} className="px-6">
                Find films &amp; series
              </Button>
            </div>
          </>
        ) : (
          <div className="mt-6">
            <Button variant="primary" onClick={pick} className="px-8 py-3 text-base">
              <FolderIcon className="h-5 w-5" />
              Pick a folder
            </Button>
          </div>
        )}

        <p className="mt-6 text-xs text-zinc-500">
          You will see every title and where it will go before anything moves.
        </p>
      </div>
    </div>
  );
}
