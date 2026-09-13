import { useCallback, useEffect, useState } from "react";
import {
  api,
  type Candidate,
  type ExecSummary,
  type MediaItem,
  type Plan,
  type ProgressEvent,
  type ScanResult,
  type Settings,
  type SkippedFile,
} from "./lib/api";
import { FolderPicker } from "./components/FolderPicker";
import { MatchDialog } from "./components/MatchDialog";
import { PlanPreview } from "./components/PlanPreview";
import { ProgressPanel } from "./components/ProgressPanel";
import { ScanResults } from "./components/ScanResults";
import { SourceHero } from "./components/SourceHero";
import { SettingsView } from "./components/Settings";
import { UndoHistory } from "./components/UndoHistory";
import { CollectionView } from "./components/Collection";
import { Button, EmptyState, Panel, Spinner } from "./components/ui";

type View = "organise" | "collection" | "history" | "settings";

const LAST_SOURCE_KEY = "cinefold.lastSource";

function Logo({ onClick }: { onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      title="Back to Organise"
      className="flex items-center gap-2.5 rounded-lg px-1.5 py-1 -ml-1.5 transition-colors hover:bg-white/5"
    >
      <div className="grid h-7 w-7 place-items-center rounded-lg bg-linear-to-br from-indigo-500 to-violet-400">
        <svg viewBox="0 0 20 20" className="h-4 w-4 text-white" fill="none">
          <path
            d="M3 6.5A1.5 1.5 0 0 1 4.5 5h3l1.2 1.5h6.8A1.5 1.5 0 0 1 17 8v6.5a1.5 1.5 0 0 1-1.5 1.5h-11A1.5 1.5 0 0 1 3 14.5v-8z"
            fill="currentColor"
          />
          <path d="M9 9.5v4l3.2-2-3.2-2z" fill="#6d6aec" />
        </svg>
      </div>
      <span className="text-sm font-semibold tracking-tight text-white">
        Cinefold
      </span>
    </button>
  );
}

export default function App() {
  const [view, setView] = useState<View>("organise");
  const [settings, setSettings] = useState<Settings | null>(null);

  const [source, setSourceState] = useState(() => {
    try {
      return localStorage.getItem(LAST_SOURCE_KEY) ?? "";
    } catch {
      return "";
    }
  });
  const setSource = useCallback((path: string) => {
    setSourceState(path);
    try {
      localStorage.setItem(LAST_SOURCE_KEY, path);
    } catch {
      // Storage can be unavailable; remembering the folder is a nicety.
    }
  }, []);
  const [scan, setScan] = useState<ScanResult | null>(null);
  const [items, setItems] = useState<MediaItem[]>([]);
  const [scanning, setScanning] = useState(false);
  const [scanProgress, setScanProgress] = useState<ProgressEvent | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);

  const [matchTarget, setMatchTarget] = useState<MediaItem | null>(null);
  const [including, setIncluding] = useState<string | null>(null);
  const [bulkBusy, setBulkBusy] = useState(false);

  const [plan, setPlan] = useState<Plan | null>(null);
  const [executing, setExecuting] = useState(false);
  const [execProgress, setExecProgress] = useState<ProgressEvent | null>(null);
  const [execErrors, setExecErrors] = useState<string[]>([]);
  const [execSummary, setExecSummary] = useState<ExecSummary | null>(null);

  // --- boot -------------------------------------------------------------
  useEffect(() => {
    api.getSettings().then(setSettings).catch(console.error);
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    api
      .onProgress((event) => {
        if (event.phase === "scan" || event.phase === "identify") {
          setScanProgress(event);
        } else {
          setExecProgress(event);
          if (event.error) {
            setExecErrors((current) => [...current, event.error as string]);
          }
        }
      })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(console.error);
    return () => unlisten?.();
  }, []);

  const replaceItem = useCallback((updated: MediaItem) => {
    setItems((current) =>
      current.map((item) => (item.id === updated.id ? updated : item)),
    );
  }, []);

  // --- scanning ---------------------------------------------------------
  async function runScan() {
    if (!source) return;
    setScanning(true);
    setScanError(null);
    setScanProgress(null);
    try {
      const result = await api.scanFolder(source);
      setScan(result);
      setItems(result.items);
    } catch (err) {
      setScanError(String(err));
    } finally {
      setScanning(false);
    }
  }

  // --- review actions ---------------------------------------------------
  async function applyMatch(candidate: Candidate) {
    if (!matchTarget) return;
    replaceItem(await api.applyMatch(matchTarget.id, candidate));
  }

  async function editEpisode(season: number | null, episode: number | null) {
    if (!matchTarget) return;
    replaceItem(await api.setItemEpisode(matchTarget.id, season, episode));
  }

  async function toggleItem(item: MediaItem) {
    const next =
      item.status === "excluded"
        ? item.identification.best && !item.identification.needsReview
          ? "ready"
          : "needsReview"
        : "excluded";
    replaceItem(await api.setItemStatus(item.id, next));
  }

  async function approveItems(targets: MediaItem[]) {
    if (targets.length === 0) return;
    setBulkBusy(true);
    try {
      const updated = await api.approveItems(targets.map((i) => i.id));
      const byId = new Map(updated.map((i) => [i.id, i]));
      setItems((current) => current.map((i) => byId.get(i.id) ?? i));
    } catch (err) {
      setScanError(String(err));
    } finally {
      setBulkBusy(false);
    }
  }

  async function excludeAll(targets: MediaItem[]) {
    setBulkBusy(true);
    try {
      const updated = await Promise.all(
        targets.map((i) => api.setItemStatus(i.id, "excluded")),
      );
      const byId = new Map(updated.map((i) => [i.id, i]));
      setItems((current) => current.map((i) => byId.get(i.id) ?? i));
    } catch (err) {
      setScanError(String(err));
    } finally {
      setBulkBusy(false);
    }
  }

  async function includeSkipped(file: SkippedFile) {
    setIncluding(file.path);
    try {
      const item = await api.includeSkipped(file.path);
      setItems((current) =>
        current.some((i) => i.id === item.id) ? current : [...current, item],
      );
      setScan((current) =>
        current
          ? {
              ...current,
              skipped: current.skipped.filter((f) => f.path !== file.path),
            }
          : current,
      );
    } catch (err) {
      setScanError(String(err));
    } finally {
      setIncluding(null);
    }
  }

  // --- plan / execute ---------------------------------------------------
  async function preview() {
    setPlan(await api.buildPlan());
  }

  async function execute() {
    if (!plan) return;
    setExecuting(true);
    setExecErrors([]);
    setExecSummary(null);
    setExecProgress(null);
    try {
      const summary = await api.executePlan(plan);
      setExecSummary(summary);

      // Drop what moved; keep anything that failed so it can be retried.
      const failedSources = new Set(
        summary.failures.map((line) => line.split(":")[0]?.trim()),
      );
      const movedIds = new Set(
        plan.entries
          .filter((entry) => entry.enabled)
          .filter((entry) =>
            entry.ops.every((op) => !failedSources.has(op.source)),
          )
          .map((entry) => entry.itemId),
      );
      setItems((current) => current.filter((item) => !movedIds.has(item.id)));
    } catch (err) {
      setExecSummary({
        runId: "",
        moved: 0,
        copied: 0,
        skipped: 0,
        decorated: 0,
        subtitles: 0,
        failures: [String(err)],
        warnings: [],
      });
    }
  }

  function closeExecution() {
    setExecuting(false);
    setExecSummary(null);
    setPlan(null);
  }

  async function saveSettings(next: Settings) {
    setSettings(await api.saveSettings(next));
  }

  // --- derived ----------------------------------------------------------
  const ready = items.filter((i) => i.status === "ready").length;
  const review = items.filter((i) => i.status === "needsReview").length;
  const needsKey = settings !== null && settings.tmdbApiKey.trim() === "";
  const needsLibrary = settings !== null && settings.libraryRoot.trim() === "";

  // A real poster from the current scan makes the icon preview meaningful.
  const samplePosterPath =
    items.find((item) => item.identification.best?.posterPath)?.identification
      .best?.posterPath ?? null;
  const samplePosterUrl = samplePosterPath
    ? `https://image.tmdb.org/t/p/w342${samplePosterPath}`
    : null;

  const navItems: { id: View; label: string }[] = [
    { id: "organise", label: "Organise" },
    { id: "collection", label: "Collection" },
    { id: "history", label: "History" },
    { id: "settings", label: "Settings" },
  ];

  return (
    <div className="flex h-full flex-col">
      <header className="flex shrink-0 items-center justify-between border-b border-white/5 px-5 py-3">
        <Logo onClick={() => setView("organise")} />
        <nav className="flex items-center gap-1">
          {navItems.map((entry) => (
            <button
              key={entry.id}
              onClick={() => setView(entry.id)}
              className={`rounded-lg px-3 py-1.5 text-sm transition-colors ${
                view === entry.id
                  ? "bg-white/10 text-white"
                  : "text-zinc-400 hover:bg-white/5 hover:text-zinc-200"
              }`}
            >
              {entry.label}
            </button>
          ))}
        </nav>
      </header>

      <main className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-5">
        {view === "settings" && settings && (
          <SettingsView
            settings={settings}
            samplePosterUrl={samplePosterUrl}
            onSave={saveSettings}
          />
        )}

        {view === "collection" && (
          <CollectionView canImport={!needsKey && !needsLibrary} />
        )}

        {view === "history" && <UndoHistory />}

        {view === "organise" && (
          <>
            {(needsKey || needsLibrary) && (
              <div className="flex items-center justify-between gap-4 rounded-xl bg-indigo-500/10 px-4 py-3 ring-1 ring-indigo-500/20">
                <p className="text-sm text-indigo-100">
                  {needsKey && needsLibrary
                    ? "Add a TMDB key and choose where your library should live to get started."
                    : needsKey
                      ? "Add a TMDB key so films and series can be recognised."
                      : "Choose where your library should live."}
                </p>
                <Button variant="primary" onClick={() => setView("settings")}>
                  Open settings
                </Button>
              </div>
            )}

            {scanError && (
              <p className="rounded-lg bg-red-500/10 px-4 py-2 text-sm text-red-300">
                {scanError}
              </p>
            )}

            {!scan ? (
              // First run: one big card, nothing else competing for attention.
              <SourceHero
                value={source}
                onChange={setSource}
                onScan={runScan}
                scanning={scanning}
                progress={scanProgress}
              />
            ) : (
              <>
                <Panel className="flex items-center gap-3 p-3">
                  <span className="shrink-0 pl-1 text-sm text-zinc-400">Folder</span>
                  <div className="min-w-0 flex-1">
                    <FolderPicker
                      value={source}
                      onChange={setSource}
                      placeholder="Folder with your films and series"
                      title="Choose the folder to organise"
                      disabled={scanning}
                    />
                  </div>
                  <Button
                    variant="primary"
                    onClick={runScan}
                    disabled={!source || scanning}
                  >
                    {scanning ? <Spinner /> : null}
                    {scanning ? "Looking" : "Look again"}
                  </Button>
                </Panel>

                {scanning ? (
                  <Panel className="flex-1">
                    <EmptyState
                      title={
                        scanProgress?.phase === "identify"
                          ? `Recognising title ${scanProgress.current} of ${scanProgress.total}`
                          : `Looking for films and episodes${scanProgress ? ` · ${scanProgress.label}` : ""}`
                      }
                    >
                      {scanProgress?.phase === "identify" && scanProgress.label}
                    </EmptyState>
                  </Panel>
                ) : (
                  <ScanResults
                    items={items}
                    skipped={scan.skipped}
                    onChangeMatch={setMatchTarget}
                    onToggle={toggleItem}
                    onInclude={includeSkipped}
                    including={including}
                    onApprove={approveItems}
                    onExcludeAll={excludeAll}
                    bulkBusy={bulkBusy}
                  />
                )}
              </>
            )}

            {scan && !scanning && (
              <div className="flex shrink-0 items-center justify-between gap-4 rounded-xl bg-zinc-900/70 px-4 py-3 ring-1 ring-white/5">
                <p className="text-sm text-zinc-400">
                  <span className="font-medium text-zinc-100">{ready}</span>{" "}
                  {ready === 1 ? "title" : "titles"} ready
                  {review > 0 && (
                    <>
                      {" · "}
                      <span className="font-medium text-amber-300">{review}</span>{" "}
                      need a look
                    </>
                  )}
                  {scan.skipped.length > 0 && ` · ${scan.skipped.length} skipped`}
                  {scan.alreadyOrganised > 0 &&
                    ` · ${scan.alreadyOrganised} already in your library`}
                  {scan.libraryExcluded && " · your library was left alone"}
                  {scan.errors.length > 0 && (
                    <span className="text-red-300">
                      {" "}
                      · {scan.errors.length} folder
                      {scan.errors.length > 1 ? "s" : ""} could not be read
                    </span>
                  )}
                </p>
                <Button
                  variant="primary"
                  onClick={preview}
                  disabled={ready === 0 || needsLibrary}
                  title={
                    needsLibrary
                      ? "Choose where your library lives in Settings first"
                      : undefined
                  }
                >
                  Preview library
                </Button>
              </div>
            )}
          </>
        )}
      </main>

      {matchTarget && (
        <MatchDialog
          item={matchTarget}
          onApply={applyMatch}
          onEditEpisode={editEpisode}
          onClose={() => setMatchTarget(null)}
        />
      )}

      {plan && !executing && settings && (
        <PlanPreview
          plan={plan}
          copyMode={settings.copyInsteadOfMove}
          onChange={setPlan}
          onExecute={execute}
          onClose={() => setPlan(null)}
        />
      )}

      {executing && (
        <ProgressPanel
          progress={execProgress}
          errors={execErrors}
          summary={execSummary}
          onClose={closeExecution}
        />
      )}
    </div>
  );
}
