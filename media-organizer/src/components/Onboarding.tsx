import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, type Settings } from "../lib/api";
import { ICON_STYLES, IconPreview, Swatches } from "./Settings";
import { Button, FolderIcon, Spinner, inputClass } from "./ui";
import logoUrl from "../assets/logo.png";

type Step = "welcome" | "library" | "tmdb" | "subtitles" | "icons";

const STEPS: Step[] = ["welcome", "library", "tmdb", "subtitles", "icons"];

type KeyState = "idle" | "checking" | "valid" | "invalid";

/**
 * First-run setup. Shown instead of the app until a TMDB key and a library
 * folder exist. Nothing is saved until the last step, so closing the app
 * halfway just shows this again next time.
 */
export function Onboarding({
  settings,
  onFinish,
}: {
  settings: Settings;
  onFinish: (settings: Settings) => Promise<void>;
}) {
  const [step, setStep] = useState<Step>("welcome");
  const [draft, setDraft] = useState<Settings>(settings);
  const [tmdbState, setTmdbState] = useState<KeyState>("idle");
  const [subdlState, setSubdlState] = useState<KeyState>("idle");
  const [finishing, setFinishing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const index = STEPS.indexOf(step);
  const next = () => setStep(STEPS[Math.min(index + 1, STEPS.length - 1)]);
  const back = () => setStep(STEPS[Math.max(index - 1, 0)]);

  function update<K extends keyof Settings>(key: K, value: Settings[K]) {
    setDraft((current) => ({ ...current, [key]: value }));
  }

  async function pickLibrary() {
    const selected = await open({
      directory: true,
      multiple: false,
      title: "Choose where your library should live",
      defaultPath: draft.libraryRoot || undefined,
    });
    if (typeof selected === "string" && selected.length > 0) {
      update("libraryRoot", selected);
    }
  }

  async function testTmdb() {
    setTmdbState("checking");
    try {
      setTmdbState((await api.validateApiKey(draft.tmdbApiKey)) ? "valid" : "invalid");
    } catch {
      setTmdbState("invalid");
    }
  }

  async function testSubdl() {
    setSubdlState("checking");
    try {
      setSubdlState((await api.validateSubdlKey(draft.subdlApiKey)) ? "valid" : "invalid");
    } catch {
      setSubdlState("invalid");
    }
  }

  async function finish() {
    setFinishing(true);
    setError(null);
    try {
      await onFinish(draft);
    } catch (err) {
      setError(String(err));
      setFinishing(false);
    }
  }

  const keyLabel = (state: KeyState) =>
    state === "valid" ? "Works" : state === "invalid" ? "Rejected" : "Test";

  return (
    <div className="flex flex-1 items-center justify-center p-6">
      <div className="w-full max-w-2xl">
        {/* step dots */}
        <div className="mb-6 flex items-center justify-center gap-2">
          {STEPS.map((s, i) => (
            <span
              key={s}
              className={`h-1.5 rounded-full transition-all ${
                i === index ? "w-8 bg-indigo-400" : i < index ? "w-3 bg-indigo-400/50" : "w-3 bg-white/15"
              }`}
            />
          ))}
        </div>

        <div className="rounded-2xl bg-zinc-900/70 p-8 ring-1 ring-white/5">
          {step === "welcome" && (
            <div className="text-center">
              <img src={logoUrl} alt="" className="mx-auto h-20 w-20" />
              <h1 className="mt-5 font-semibold text-2xl text-white">Welcome to Cinefold</h1>
              <p className="mx-auto mt-3 max-w-md text-zinc-400">
                Point it at a messy folder of films and series. It works out
                what everything is, files it neatly with posters and subtitles,
                and shows you every change before anything moves.
              </p>
              <p className="mt-2 text-sm text-zinc-500">
                Setup takes about a minute. Two things are needed; two are optional.
              </p>
              <Button variant="primary" onClick={next} className="mt-8 px-8 py-3 text-base">
                Get started
              </Button>
            </div>
          )}

          {step === "library" && (
            <div>
              <p className="text-xs font-semibold uppercase tracking-wide text-indigo-300">Step 1 of 4</p>
              <h2 className="mt-1 text-xl font-semibold text-white">Where should your library live?</h2>
              <p className="mt-2 text-sm text-zinc-400">
                Organised films and series go here. Cinefold creates{" "}
                <span className="text-zinc-200">Movies</span> and{" "}
                <span className="text-zinc-200">TV Shows</span> folders inside it.
                An empty folder is a good start.
              </p>
              <div className="mt-6 flex items-center gap-2">
                <div
                  className="min-w-0 flex-1 truncate rounded-lg bg-zinc-950/60 px-3 py-2.5 text-sm ring-1 ring-white/10"
                  title={draft.libraryRoot}
                >
                  {draft.libraryRoot ? (
                    <span className="text-zinc-200">{draft.libraryRoot}</span>
                  ) : (
                    <span className="text-zinc-600">No folder chosen yet</span>
                  )}
                </div>
                <Button onClick={pickLibrary}>
                  <FolderIcon />
                  Browse
                </Button>
              </div>
            </div>
          )}

          {step === "tmdb" && (
            <div>
              <p className="text-xs font-semibold uppercase tracking-wide text-indigo-300">Step 2 of 4</p>
              <h2 className="mt-1 text-xl font-semibold text-white">A free TMDB key</h2>
              <p className="mt-2 text-sm text-zinc-400">
                This is how Cinefold recognises titles and fetches posters. It's
                free and takes a minute: sign up at{" "}
                <span className="text-zinc-200">themoviedb.org</span>, open{" "}
                <span className="text-zinc-200">Settings &rarr; API</span>, and copy
                the <span className="text-zinc-200">API Key (v3 auth)</span>.
              </p>
              <div className="mt-6 flex gap-2">
                <input
                  value={draft.tmdbApiKey}
                  onChange={(e) => {
                    update("tmdbApiKey", e.target.value.trim());
                    setTmdbState("idle");
                  }}
                  placeholder="Paste your key here"
                  className={`${inputClass} mt-0 font-mono`}
                  type="password"
                  autoComplete="off"
                  spellCheck={false}
                  autoFocus
                />
                <Button onClick={testTmdb} disabled={tmdbState === "checking" || !draft.tmdbApiKey}>
                  {tmdbState === "checking" && <Spinner />}
                  {keyLabel(tmdbState)}
                </Button>
              </div>
              {tmdbState === "invalid" && (
                <p className="mt-2 text-sm text-red-300">
                  TMDB didn't accept that key. Check you copied the whole thing.
                </p>
              )}
              <Button
                variant="ghost"
                onClick={() => openUrl("https://www.themoviedb.org/settings/api")}
                className="mt-3 px-0"
              >
                Open the TMDB API page &rarr;
              </Button>
            </div>
          )}

          {step === "subtitles" && (
            <div>
              <p className="text-xs font-semibold uppercase tracking-wide text-indigo-300">
                Step 3 of 4 &middot; Optional
              </p>
              <h2 className="mt-1 text-xl font-semibold text-white">Subtitles, when you want them</h2>
              <p className="mt-2 text-sm text-zinc-400">
                Cinefold can fetch a missing subtitle for any film or episode with
                one click. That needs a free key from{" "}
                <span className="text-zinc-200">subdl.com</span> (your profile &rarr; API Key).
                Nothing downloads on its own. You can add this later in Settings.
              </p>
              <div className="mt-6 flex gap-2">
                <input
                  value={draft.subdlApiKey}
                  onChange={(e) => {
                    update("subdlApiKey", e.target.value.trim());
                    setSubdlState("idle");
                  }}
                  placeholder="Paste a SubDL key, or skip"
                  className={`${inputClass} mt-0 font-mono`}
                  type="password"
                  autoComplete="off"
                  spellCheck={false}
                />
                <Button onClick={testSubdl} disabled={subdlState === "checking" || !draft.subdlApiKey}>
                  {subdlState === "checking" && <Spinner />}
                  {keyLabel(subdlState)}
                </Button>
              </div>
            </div>
          )}

          {step === "icons" && (
            <div>
              <p className="text-xs font-semibold uppercase tracking-wide text-indigo-300">
                Step 4 of 4 &middot; Optional
              </p>
              <h2 className="mt-1 text-xl font-semibold text-white">How should your folders look?</h2>
              <p className="mt-2 text-sm text-zinc-400">
                Every organised folder gets its poster as the icon. Pick a style
                and colour, or keep the defaults and change it any time in Settings.
              </p>
              <div className="mt-5">
                <IconPreview draft={draft} posterUrl={null} />
              </div>
              <div className="mt-4 grid grid-cols-3 gap-2">
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
                    <span className="block text-sm font-medium text-zinc-100">{style.label}</span>
                    <span className="block text-xs text-zinc-500">{style.hint}</span>
                  </button>
                ))}
              </div>
              <div className="mt-4">
                <Swatches value={draft.folderTint} onChange={(c) => update("folderTint", c)} />
              </div>
            </div>
          )}

          {error && (
            <p className="mt-4 rounded-lg bg-red-500/10 px-3 py-2 text-sm text-red-300">{error}</p>
          )}

          {/* footer nav (welcome has its own button) */}
          {step !== "welcome" && (
            <div className="mt-8 flex items-center justify-between">
              <Button variant="ghost" onClick={back}>
                Back
              </Button>
              <div className="flex items-center gap-2">
                {(step === "subtitles" || step === "icons") && (
                  <Button variant="ghost" onClick={step === "icons" ? finish : next} disabled={finishing}>
                    {step === "icons" ? "Skip and finish" : "Skip for now"}
                  </Button>
                )}
                {step === "icons" ? (
                  <Button variant="primary" onClick={finish} disabled={finishing}>
                    {finishing && <Spinner />}
                    Finish
                  </Button>
                ) : (
                  <Button
                    variant="primary"
                    onClick={next}
                    disabled={
                      (step === "library" && !draft.libraryRoot) ||
                      (step === "tmdb" && !draft.tmdbApiKey)
                    }
                  >
                    Continue
                  </Button>
                )}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
