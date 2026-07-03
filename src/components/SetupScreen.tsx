import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Brain, Loader2, Check, FolderOpen } from "lucide-react";
import { downloadModel, installModelFile, listModels, type ModelInfo } from "@/lib/api";
import { open } from "@tauri-apps/plugin-dialog";
import { cn } from "@/lib/utils";

interface DownloadProgress { downloaded_mb: number; total_mb: number; percent: number; }
type Step = "pick" | "downloading" | "copying";

interface Props { onDone: () => void; }

export function SetupScreen({ onDone }: Props) {
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [step, setStep] = useState<Step>("pick");
  const [dlProgress, setDlProgress] = useState<DownloadProgress | null>(null);
  const [copyProgress, setCopyProgress] = useState(0);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listModels().then((ms) => {
      setModels(ms);
      const rec = ms.find((m) => m.recommended) ?? ms[0];
      if (rec) setSelectedId(rec.id);
    });
  }, []);

  useEffect(() => {
    if (step !== "downloading") return;
    const u = listen<DownloadProgress>("download-progress", (e) => setDlProgress(e.payload));
    return () => { u.then((fn) => fn()); };
  }, [step]);

  useEffect(() => {
    if (step !== "copying") return;
    const u = listen<number>("install-progress", (e) => setCopyProgress(e.payload));
    return () => { u.then((fn) => fn()); };
  }, [step]);

  async function handleDownload() {
    if (!selectedId) return;
    setError(null);
    setStep("downloading");
    try {
      await downloadModel(selectedId);
      onDone();
    } catch (e) {
      setError(String(e));
      setStep("pick");
    }
  }

  async function handleLocalFile() {
    if (!selectedId) return;
    const file = await open({ filters: [{ name: "GGUF", extensions: ["gguf"] }], multiple: false });
    if (!file || typeof file !== "string") return;
    setError(null);
    setStep("copying");
    try {
      await installModelFile(file, selectedId);
      onDone();
    } catch (e) {
      setError(String(e));
      setStep("pick");
    }
  }

  return (
    <div className="flex h-screen w-screen flex-col items-center justify-center gap-6 bg-[var(--color-bg)] text-[var(--color-text)]">
      <Brain className="size-12 text-[var(--color-accent)]" />

      <div className="text-center">
        <h1 className="text-lg font-semibold">Lucid — Modèle IA</h1>
        <p className="mt-1 text-sm text-[var(--color-muted)]">
          {step === "pick" && "Choisis un modèle et télécharge-le en un clic"}
          {step === "downloading" && "Téléchargement en cours…"}
          {step === "copying" && "Installation en cours…"}
        </p>
      </div>

      {/* Sélecteur */}
      {step === "pick" && (
        <div className="flex w-80 flex-col gap-2" style={{ maxHeight: "calc(100vh - 200px)" }}>
          {models.length === 0
            ? <Loader2 className="mx-auto size-5 animate-spin text-[var(--color-muted)]" />
            : (
              <div className="flex-1 overflow-y-auto space-y-2 pr-1">
                {models.map((m) => (
                  <button
                    key={m.id}
                    onClick={() => setSelectedId(m.id)}
                    className={cn(
                      "flex w-full flex-col gap-0.5 rounded-xl border px-4 py-3 text-left transition-colors",
                      selectedId === m.id
                        ? "border-[var(--color-accent)] bg-[var(--color-accent-soft)]"
                        : "border-[var(--color-border)] bg-[var(--color-surface)] hover:border-[var(--color-accent)]/50",
                    )}
                  >
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-semibold">{m.name}</span>
                      {m.recommended && (
                        <span className="rounded-full bg-[var(--color-accent)]/20 px-1.5 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
                          Recommandé
                        </span>
                      )}
                      {selectedId === m.id && <Check className="ml-auto size-4 text-[var(--color-accent)]" />}
                    </div>
                    <p className="text-xs text-[var(--color-muted)]">{m.size_gb.toFixed(1)} Go · min {m.min_ram_gb} Go RAM</p>
                  </button>
                ))}
              </div>
            )
          }

          {models.length > 0 && (
            <div className="flex flex-col gap-1.5 pt-1">
              <button
                onClick={handleDownload}
                disabled={!selectedId}
                className="w-full rounded-xl bg-[var(--color-accent)] px-4 py-2.5 text-sm font-semibold text-white transition-opacity disabled:opacity-50 hover:opacity-90"
              >
                Télécharger
              </button>
              <button
                onClick={handleLocalFile}
                disabled={!selectedId}
                className="flex w-full items-center justify-center gap-1.5 text-xs text-[var(--color-muted)] hover:text-[var(--color-text)] transition-colors disabled:opacity-40"
              >
                <FolderOpen className="size-3.5" /> J'ai déjà le fichier .gguf
              </button>
            </div>
          )}

          {error && (
            <div className="rounded-lg border border-[var(--color-err)]/30 bg-[var(--color-err)]/10 px-3 py-2.5 text-xs text-[var(--color-err)]">
              {error}
            </div>
          )}
        </div>
      )}

      {/* Progress téléchargement */}
      {step === "downloading" && (
        <div className="w-72 space-y-2">
          <div className="h-2 w-full overflow-hidden rounded-full bg-[var(--color-surface-2)]">
            <div
              className="h-full rounded-full bg-[var(--color-accent)] transition-all duration-300"
              style={{ width: `${dlProgress?.percent ?? 0}%` }}
            />
          </div>
          <p className="text-center text-xs text-[var(--color-muted)]">
            {dlProgress
              ? `${dlProgress.downloaded_mb.toFixed(0)} Mo / ${dlProgress.total_mb.toFixed(0)} Mo · ${dlProgress.percent}%`
              : "Connexion…"}
          </p>
        </div>
      )}

      {/* Progress copie */}
      {step === "copying" && (
        <div className="w-72 space-y-2">
          <div className="h-2 w-full overflow-hidden rounded-full bg-[var(--color-surface-2)]">
            <div
              className="h-full rounded-full bg-[var(--color-accent)] transition-all duration-200"
              style={{ width: `${copyProgress}%` }}
            />
          </div>
          <p className="text-center text-xs text-[var(--color-muted)]">
            {copyProgress > 0 ? `${copyProgress}%` : "Copie…"}
          </p>
        </div>
      )}
    </div>
  );
}
