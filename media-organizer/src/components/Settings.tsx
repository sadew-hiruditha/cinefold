import { useEffect, useRef, useState } from "react";
import { api, type IconStyle, type Settings } from "../lib/api";
import { FolderPicker } from "./FolderPicker";
import { Button, Field, Panel, Spinner, Toggle, inputClass } from "./ui";

type KeyState = "idle" | "checking" | "valid" | "invalid";

function keyLabel(state: KeyState): string {
  return state === "valid" ? "Works" : state === "invalid" ? "Rejected" : "Test";
}
type SettingsTab = "icons" | "library" | "advanced";

const TABS: { id: SettingsTab; label: string; hint: string }[] = [
  { id: "icons", label: "Folder icons", hint: "How organised folders look" },
  { id: "library", label: "Library", hint: "Where files go, TMDB key" },
  { id: "advanced", label: "Advanced", hint: "Naming, scanning, safety" },
];

const TINT_PRESETS = [
  "#6366f1",
  "#8b5cf6",
  "#ec4899",
  "#ef4444",
  "#f59e0b",
  "#10b981",
  "#06b6d4",
  "#3b82f6",
  "#71717a",
];

const ICON_STYLES: { id: IconStyle; label: string; hint: string }[] = [
  { id: "card", label: "Card", hint: "Poster on a coloured tile" },
  { id: "folder", label: "Folder", hint: "Folder shape with the poster inside" },
  { id: "poster", label: "Poster", hint: "Artwork only, optional frame" },
];

function Swatches({
  value,
  onChange,
}: {
  value: string;
  onChange: (colour: string) => void;
}) {
  return (
    <div className="mt-2 flex flex-wrap items-center gap-2">
      {TINT_PRESETS.map((colour) => (
        <button
          key={colour}
          type="button"
          onClick={() => onChange(colour)}
          title={colour}
          className={`h-7 w-7 rounded-full ring-2 ring-offset-2 ring-offset-zinc-900 transition-transform hover:scale-110 ${
            value.toLowerCase() === colour ? "ring-white" : "ring-transparent"
          }`}
          style={{ backgroundColor: colour }}
        />
      ))}
      <input
        value={value}
        onChange={(event) => onChange(event.target.value)}
        className={`${inputClass} mt-0 w-28 font-mono`}
        spellCheck={false}
      />
    </div>
  );
}

/** Live render of the folder icon at three sizes, debounced as sliders move. */
function IconPreview({
  draft,
  posterUrl,
}: {
  draft: Settings;
  posterUrl: string | null;
}) {
  const [frames, setFrames] = useState<string[]>([]);
  const [tvFrames, setTvFrames] = useState<string[]>([]);
  const [failed, setFailed] = useState(false);
  const timer = useRef<number | undefined>(undefined);

  const {
    iconStyle,
    iconBorder,
    iconCorner,
    iconFill,
    folderTint,
    tvTint,
    separateTvTint,
  } = draft;

  useEffect(() => {
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      const film = api.previewIcon(draft, posterUrl, "movie");
      const series = separateTvTint
        ? api.previewIcon(draft, posterUrl, "tv")
        : Promise.resolve([]);
      Promise.all([film, series])
        .then(([a, b]) => {
          setFrames(a);
          setTvFrames(b);
          setFailed(false);
        })
        .catch(() => setFailed(true));
    }, 120);
    return () => window.clearTimeout(timer.current);
    // Only re-render when something that affects the drawing changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    iconStyle,
    iconBorder,
    iconCorner,
    iconFill,
    folderTint,
    tvTint,
    separateTvTint,
    posterUrl,
  ]);

  const strip = (urls: string[], label: string) => (
    <div className="flex flex-col items-center gap-2">
      <div className="flex items-end gap-4">
        <img src={urls[0]} alt={`${label} icon, large`} className="h-32 w-32" />
        <img src={urls[1]} alt={`${label} icon, medium`} className="h-12 w-12" />
        <img src={urls[2]} alt={`${label} icon, small`} className="h-4 w-4" />
      </div>
      {separateTvTint && <span className="text-xs text-zinc-500">{label}</span>}
    </div>
  );

  return (
    <div className="flex items-end gap-8 rounded-lg bg-zinc-950/60 px-5 py-4 ring-1 ring-white/5">
      {frames.length === 3 ? (
        <>
          {strip(frames, "Films")}
          {separateTvTint && tvFrames.length === 3 && strip(tvFrames, "Series")}
        </>
      ) : (
        <div className="flex h-32 items-center gap-2 text-sm text-zinc-500">
          {failed ? "Preview unavailable" : <Spinner />}
        </div>
      )}
      <p className="ml-auto max-w-45 self-center text-xs text-zinc-500">
        {posterUrl
          ? "Using a poster from your last scan."
          : "Scan a folder and a real poster will be used here."}
      </p>
    </div>
  );
}

export function SettingsView({
  settings,
  samplePosterUrl,
  onSave,
}: {
  settings: Settings;
  samplePosterUrl: string | null;
  onSave: (settings: Settings) => Promise<void>;
}) {
  const [draft, setDraft] = useState<Settings>(settings);
  const [keyState, setKeyState] = useState<KeyState>("idle");
  const [subdlState, setSubdlState] = useState<KeyState>("idle");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [paths, setPaths] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [reapplying, setReapplying] = useState(false);
  const [reapplyResult, setReapplyResult] = useState<string | null>(null);
  // Icons first: it is the visual, everyday setting. The rest is setup.
  const [tab, setTab] = useState<SettingsTab>("icons");

  useEffect(() => {
    setDraft(settings);
  }, [settings]);

  useEffect(() => {
    api.appPaths().then(setPaths).catch(() => setPaths({}));
  }, []);

  function update<K extends keyof Settings>(key: K, value: Settings[K]) {
    setDraft((current) => ({ ...current, [key]: value }));
    setSaved(false);
  }

  async function checkKey() {
    setKeyState("checking");
    try {
      const ok = await api.validateApiKey(draft.tmdbApiKey);
      setKeyState(ok ? "valid" : "invalid");
    } catch {
      setKeyState("invalid");
    }
  }

  async function checkSubdlKey() {
    setSubdlState("checking");
    try {
      const ok = await api.validateSubdlKey(draft.subdlApiKey);
      setSubdlState(ok ? "valid" : "invalid");
    } catch {
      setSubdlState("invalid");
    }
  }

  async function reapplyIcons() {
    setReapplying(true);
    setReapplyResult(null);
    try {
      // Icons are generated from saved settings, so persist the draft first.
      await onSave(draft);
      setSaved(true);
      const report = await api.reapplyLibraryIcons();
      setReapplyResult(
        report.failures.length === 0
          ? `Updated ${report.updated} folder${report.updated === 1 ? "" : "s"}.`
          : `Updated ${report.updated}, ${report.failures.length} failed: ${report.failures[0]}`,
      );
    } catch (err) {
      setReapplyResult(String(err));
    } finally {
      setReapplying(false);
    }
  }

  async function save() {
    setSaving(true);
    setError(null);
    try {
      await onSave(draft);
      setSaved(true);
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }

  const listToText = (list: string[]) => list.join(", ");
  const textToList = (text: string) =>
    text
      .split(/[,\s]+/)
      .map((s) => s.trim().toLowerCase().replace(/^\./, ""))
      .filter(Boolean);

  return (
    <div className="mx-auto w-full max-w-3xl space-y-4 pb-24">
      <div className="grid grid-cols-3 gap-2">
        {TABS.map((entry) => (
          <button
            key={entry.id}
            onClick={() => setTab(entry.id)}
            className={`rounded-xl px-4 py-3 text-left ring-1 transition-colors ${
              tab === entry.id
                ? "bg-indigo-500/15 ring-indigo-400/50"
                : "bg-zinc-900/70 ring-white/5 hover:bg-white/5"
            }`}
          >
            <span className="block text-sm font-medium text-zinc-100">
              {entry.label}
            </span>
            <span className="block text-xs text-zinc-500">{entry.hint}</span>
          </button>
        ))}
      </div>

      {tab === "icons" && (
        <Panel className="space-y-5 p-5">
          <IconPreview draft={draft} posterUrl={samplePosterUrl} />

          <Toggle
            checked={draft.setFolderIcons}
            onChange={(value) => update("setFolderIcons", value)}
            label="Put the poster on the folder"
            hint="Turn this off and folders keep the normal Windows look."
          />

          <Field label="Style">
            <div className="mt-2 grid grid-cols-3 gap-2">
              {ICON_STYLES.map((style) => (
                <button
                  key={style.id}
                  type="button"
                  onClick={() => update("iconStyle", style.id)}
                  className={`rounded-lg px-3 py-2.5 text-left ring-1 transition-colors ${
                    draft.iconStyle === style.id
                      ? "bg-indigo-500/15 ring-indigo-400/50"
                      : "bg-zinc-950/40 ring-white/10 hover:bg-white/5"
                  }`}
                >
                  <span className="block text-sm font-medium text-zinc-100">
                    {style.label}
                  </span>
                  <span className="block text-xs text-zinc-500">{style.hint}</span>
                </button>
              ))}
            </div>
          </Field>

          <div className="grid gap-4 sm:grid-cols-2">
            <Field label={`Border · ${draft.iconBorder}%`}>
              <input
                type="range"
                min={0}
                max={20}
                step={1}
                value={draft.iconBorder}
                onChange={(event) => update("iconBorder", Number(event.target.value))}
                className="mt-2 w-full accent-indigo-500"
              />
            </Field>
            <Field label={`Rounded corners · ${draft.iconCorner}%`}>
              <input
                type="range"
                min={0}
                max={30}
                step={1}
                value={draft.iconCorner}
                onChange={(event) => update("iconCorner", Number(event.target.value))}
                className="mt-2 w-full accent-indigo-500"
              />
            </Field>
          </div>

          <Toggle
            checked={draft.iconFill}
            onChange={(value) => update("iconFill", value)}
            label="Fill the whole shape with the poster"
            hint="Trims the top and bottom of the artwork so there are no gaps at the sides."
          />

          <Field label={draft.separateTvTint ? "Films colour" : "Colour"}>
            <Swatches
              value={draft.folderTint}
              onChange={(colour) => update("folderTint", colour)}
            />
          </Field>

          <Toggle
            checked={draft.separateTvTint}
            onChange={(value) => update("separateTvTint", value)}
            label="Different colour for series"
            hint="Films and series get their own colour so the two are easy to tell apart."
          />

          {draft.separateTvTint && (
            <Field label="Series colour">
              <Swatches
                value={draft.tvTint}
                onChange={(colour) => update("tvTint", colour)}
              />
            </Field>
          )}

          <div className="flex items-center justify-between gap-4 rounded-lg bg-white/[0.03] px-3 py-2.5">
            <div className="min-w-0">
              <p className="text-sm text-zinc-200">Update folders you already organised</p>
              <p className="text-xs text-zinc-500">
                {reapplyResult ?? "Redraws every icon in your library with the look above."}
              </p>
            </div>
            <Button
              onClick={reapplyIcons}
              disabled={reapplying || draft.libraryRoot.trim() === ""}
            >
              {reapplying && <Spinner />}
              Re-apply icons
            </Button>
          </div>

          <div className="flex items-center justify-between gap-4 rounded-lg bg-white/[0.03] px-3 py-2.5">
            <div>
              <p className="text-sm text-zinc-200">Icons not showing up?</p>
              <p className="text-xs text-zinc-500">Ask Windows to look again.</p>
            </div>
            <Button onClick={() => api.refreshIconCache()}>Refresh</Button>
          </div>
        </Panel>
      )}

      {tab === "library" && (
        <Panel className="space-y-5 p-5">
          <Field
            label="Library folder"
            hint="Your organised films and series end up in here."
          >
            <div className="mt-1.5">
              <FolderPicker
                value={draft.libraryRoot}
                onChange={(path) => update("libraryRoot", path)}
                title="Choose the library folder"
              />
            </div>
          </Field>

          <Field
            label="TMDB API key"
            hint={
              <>
                Needed to recognise titles and fetch posters. Free at{" "}
                <span className="text-zinc-300">themoviedb.org &rarr; Settings &rarr; API</span>.
                It never leaves this computer.
              </>
            }
          >
            <div className="mt-1.5 flex gap-2">
              <input
                value={draft.tmdbApiKey}
                onChange={(event) => {
                  update("tmdbApiKey", event.target.value.trim());
                  setKeyState("idle");
                }}
                className={`${inputClass} mt-0 font-mono`}
                type="password"
                autoComplete="off"
                spellCheck={false}
              />
              <Button
                onClick={checkKey}
                disabled={keyState === "checking" || draft.tmdbApiKey === ""}
              >
                {keyState === "checking" && <Spinner />}
                {keyState === "valid"
                  ? "Works"
                  : keyState === "invalid"
                    ? "Rejected"
                    : "Test"}
              </Button>
            </div>
          </Field>

          <Field
            label="SubDL API key"
            hint={
              <>
                Optional. Fetches subtitles for titles that arrive without one.
                Free at{" "}
                <span className="text-zinc-300">subdl.com &rarr; your profile &rarr; API Key</span>.
              </>
            }
          >
            <div className="mt-1.5 flex gap-2">
              <input
                value={draft.subdlApiKey}
                onChange={(event) => {
                  update("subdlApiKey", event.target.value.trim());
                  setSubdlState("idle");
                }}
                className={`${inputClass} mt-0 font-mono`}
                type="password"
                autoComplete="off"
                spellCheck={false}
              />
              <Button
                onClick={checkSubdlKey}
                disabled={subdlState === "checking" || draft.subdlApiKey === ""}
              >
                {subdlState === "checking" && <Spinner />}
                {keyLabel(subdlState)}
              </Button>
            </div>
          </Field>

          <Toggle
            checked={draft.fetchSubtitles}
            onChange={(value) => update("fetchSubtitles", value)}
            label="Auto-fetch subtitles for every title (off by default)"
            hint="Off means nothing downloads on its own - use Get subtitles on a film or series in your Collection whenever you actually want one."
          />

          <Field
            label="Subtitle languages"
            hint="Two-letter codes like en, fr. Local subtitles in other languages are still kept; fetching uses English if this is empty."
          >
            <input
              value={listToText(draft.preferredLanguages)}
              onChange={(event) =>
                update("preferredLanguages", textToList(event.target.value))
              }
              className={inputClass}
              placeholder="all languages"
            />
          </Field>
        </Panel>
      )}

      {tab === "advanced" && (
        <>
          <Panel className="space-y-5 p-5">
            <h3 className="text-sm font-semibold text-zinc-100">Naming</h3>
            <Field
              label="Films"
              hint={
                <>
                  <code>{"{title} {year} {edition} {resolution}"}</code> &middot; use{" "}
                  <code>/</code> for folders
                </>
              }
            >
              <input
                value={draft.movieTemplate}
                onChange={(event) => update("movieTemplate", event.target.value)}
                className={`${inputClass} font-mono`}
                spellCheck={false}
              />
            </Field>
            <Field
              label="Series"
              hint={
                <>
                  also <code>{"{show} {season:02} {episode:02} {episode_title}"}</code>
                </>
              }
            >
              <input
                value={draft.tvTemplate}
                onChange={(event) => update("tvTemplate", event.target.value)}
                className={`${inputClass} font-mono`}
                spellCheck={false}
              />
            </Field>
          </Panel>

          <Panel className="space-y-5 p-5">
            <h3 className="text-sm font-semibold text-zinc-100">Matching</h3>
            <Field
              label={`Accept automatically above ${Math.round(draft.matchThreshold * 100)}% confidence`}
              hint="Anything less certain waits for you in Needs review."
            >
              <input
                type="range"
                min={0.5}
                max={0.95}
                step={0.01}
                value={draft.matchThreshold}
                onChange={(event) =>
                  update("matchThreshold", Number(event.target.value))
                }
                className="mt-2 w-full accent-indigo-500"
              />
            </Field>
          </Panel>

          <Panel className="space-y-5 p-5">
            <h3 className="text-sm font-semibold text-zinc-100">Scanning</h3>
            <Field label="Ignore videos smaller than (MB)">
              <input
                type="number"
                min={0}
                value={draft.minFileSizeMb}
                onChange={(event) =>
                  update("minFileSizeMb", Math.max(0, Number(event.target.value)))
                }
                className={`${inputClass} w-32`}
              />
            </Field>
            <Field label="Video extensions">
              <input
                value={listToText(draft.videoExtensions)}
                onChange={(event) =>
                  update("videoExtensions", textToList(event.target.value))
                }
                className={`${inputClass} font-mono`}
              />
            </Field>
            <Field label="Subtitle extensions">
              <input
                value={listToText(draft.subtitleExtensions)}
                onChange={(event) =>
                  update("subtitleExtensions", textToList(event.target.value))
                }
                className={`${inputClass} font-mono`}
              />
            </Field>
            <Toggle
              checked={draft.skipSamples}
              onChange={(value) => update("skipSamples", value)}
              label="Skip samples and trailers"
            />
          </Panel>

          <Panel className="space-y-5 p-5">
            <h3 className="text-sm font-semibold text-zinc-100">Safety</h3>
            <Toggle
              checked={draft.copyInsteadOfMove}
              onChange={(value) => update("copyInsteadOfMove", value)}
              label="Copy instead of move"
              hint="Leaves the originals where they are. Uses twice the space."
            />
            <Toggle
              checked={draft.deepVerify}
              onChange={(value) => update("deepVerify", value)}
              label="Double-check copies before deleting the original"
              hint="Only when moving between drives. Slower but safest."
            />
            <Field label="Undo history to keep">
              <input
                type="number"
                min={1}
                max={200}
                value={draft.keepUndoRuns}
                onChange={(event) =>
                  update("keepUndoRuns", Math.max(1, Number(event.target.value)))
                }
                className={`${inputClass} w-32`}
              />
            </Field>
          </Panel>

          {Object.keys(paths).length > 0 && (
            <Panel className="p-5">
              <h3 className="text-sm font-semibold text-zinc-100">On this machine</h3>
              <dl className="mt-3 space-y-1.5 font-mono text-xs text-zinc-500">
                {Object.entries(paths).map(([label, value]) => (
                  <div key={label} className="grid grid-cols-[64px_1fr] gap-2">
                    <dt className="text-zinc-400">{label}</dt>
                    <dd className="truncate" title={value}>
                      {value}
                    </dd>
                  </div>
                ))}
              </dl>
            </Panel>
          )}
        </>
      )}

      <div className="fixed inset-x-0 bottom-0 border-t border-white/5 bg-zinc-950/80 px-6 py-3 backdrop-blur">
        <div className="mx-auto flex max-w-3xl items-center justify-end gap-3">
          {error && <span className="mr-auto text-sm text-red-300">{error}</span>}
          {saved && !error && (
            <span className="mr-auto text-sm text-emerald-300">Saved</span>
          )}
          <Button variant="primary" onClick={save} disabled={saving}>
            {saving && <Spinner />}
            Save
          </Button>
        </div>
      </div>
    </div>
  );
}
