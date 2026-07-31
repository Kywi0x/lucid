import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Brain, Loader2, WifiOff, RefreshCw } from "lucide-react";
import { aiBootstrap } from "@/lib/api";

interface DownloadProgress { downloaded_mb: number; total_mb: number; percent: number; }
interface BootstrapStep { step: number; total: number; label: string; }

interface Props {
  /** Appelé quand le modèle de génération est prêt (l'app peut démarrer). */
  onDone: () => void;
}

/**
 * 1er lancement : télécharge AUTOMATIQUEMENT le modèle IA recommandé + le moteur
 * d'embedding — aucun choix à faire (le choix manuel vit dans les Réglages).
 * Hors-ligne : écran d'attente + Réessayer (l'app ne démarre pas sans IA,
 * décision Liam 2026-07-30). Le détail de progression suit les events Rust.
 */
export function SetupScreen({ onDone }: Props) {
  const [step, setStep] = useState<BootstrapStep | null>(null);
  const [dlProgress, setDlProgress] = useState<DownloadProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Garde contre un double-lancement (StrictMode monte l'effet deux fois en dev).
  const runningRef = useRef(false);

  useEffect(() => {
    const unStep = listen<BootstrapStep>("bootstrap-step", (e) => { setStep(e.payload); setDlProgress(null); });
    const unDl = listen<DownloadProgress>("download-progress", (e) => setDlProgress(e.payload));
    return () => { unStep.then((f) => f()); unDl.then((f) => f()); };
  }, []);

  async function run() {
    if (runningRef.current) return;
    runningRef.current = true;
    setError(null);
    try {
      await aiBootstrap();
      onDone();
    } catch (e) {
      setError(String(e));
    } finally {
      runningRef.current = false;
    }
  }

  useEffect(() => {
    run();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const stepLabel = step ? `${step.label}${step.total > 1 ? ` · ${step.step}/${step.total}` : ""}` : "Préparation…";

  return (
    <div className="flex h-screen w-screen flex-col items-center justify-center gap-6 bg-[var(--color-bg)] text-[var(--color-text)]">
      <Brain className="size-12 text-[var(--color-accent)]" />

      <div className="text-center">
        <h1 className="text-lg font-semibold">Préparation de Lucid</h1>
        <p className="mt-1 text-sm text-[var(--color-muted)]">
          {error ? "Connexion requise" : "Installation de l'IA locale, une seule fois…"}
        </p>
      </div>

      {error ? (
        <div className="flex w-80 flex-col items-center gap-3">
          <div className="flex items-center gap-2 text-[var(--color-muted)]">
            <WifiOff className="size-5" />
            <span className="text-sm">Impossible de télécharger l'IA</span>
          </div>
          <p className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] px-3 py-2.5 text-center text-xs text-[var(--color-muted)]">
            Une connexion internet est nécessaire au premier lancement pour
            installer le modèle (une seule fois). Vérifie ta connexion puis
            réessaie.
          </p>
          <p className="max-w-full truncate text-[10px] text-[var(--color-muted)]/70" title={error}>{error}</p>
          <button
            onClick={run}
            className="flex items-center gap-1.5 rounded-xl bg-[var(--color-accent)] px-4 py-2.5 text-sm font-semibold text-white transition-opacity hover:opacity-90"
          >
            <RefreshCw className="size-4" /> Réessayer
          </button>
        </div>
      ) : (
        <div className="w-72 space-y-2">
          <div className="h-2 w-full overflow-hidden rounded-full bg-[var(--color-surface-2)]">
            <div
              className="h-full rounded-full bg-[var(--color-accent)] transition-all duration-300"
              style={{ width: `${dlProgress?.percent ?? 0}%` }}
            />
          </div>
          <p className="flex items-center justify-center gap-1.5 text-center text-xs text-[var(--color-muted)]">
            <Loader2 className="size-3.5 animate-spin" />
            {dlProgress
              ? `${stepLabel} · ${dlProgress.downloaded_mb.toFixed(0)} / ${dlProgress.total_mb.toFixed(0)} Mo`
              : stepLabel}
          </p>
        </div>
      )}
    </div>
  );
}
