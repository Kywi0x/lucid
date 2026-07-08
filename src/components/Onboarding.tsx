import { useState } from "react";
import { Brain, Check, Copy, Sparkles } from "lucide-react";
import type { ConnectorStatus } from "@/lib/types";
import { ConnectorLogo, AiClientsSection } from "@/components/SettingsModal";
import { cn } from "@/lib/utils";

const EXAMPLE_PROMPT = "Qu'est-ce qu'il y a dans mon second cerveau Lucid ? Fais-moi un tour d'horizon.";

interface Props {
  phase: "sources" | "connect";
  connectors: ConnectorStatus[];
  /** Ouvre les Paramètres (section Sources) par-dessus l'onboarding. */
  onOpenSettings: () => void;
  /** Lance la génération du cerveau (écran 2 = le genesis existant). */
  onGenerate: () => void;
  /** Termine (ou saute) l'onboarding. */
  onDone: () => void;
  /** Seed un graphe de démo et entre dans la carte pour explorer sans connecteur. */
  onSeedDemo: () => void;
}

function StepDots({ current }: { current: number }) {
  return (
    <div className="flex items-center justify-center gap-1.5">
      {[0, 1, 2].map((i) => (
        <span
          key={i}
          className={cn(
            "size-1.5 rounded-full transition-colors",
            i <= current ? "bg-[var(--color-accent)]" : "bg-[var(--color-border)]",
          )}
        />
      ))}
    </div>
  );
}

/** Onboarding premier lancement : Sources → génération (genesis) → Brancher son IA. */
export function Onboarding({ phase, connectors, onOpenSettings, onGenerate, onDone, onSeedDemo }: Props) {
  const [copied, setCopied] = useState(false);
  const hasSource = connectors.some((c) => c.connected && c.conversation_count > 0);

  function copyPrompt() {
    navigator.clipboard.writeText(EXAMPLE_PROMPT).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  }

  return (
    <div className="absolute inset-0 z-40 flex items-center justify-center bg-[var(--color-bg)]/95">
      <div className="panel w-[560px] max-w-[calc(100vw-2rem)] overflow-hidden rounded-2xl">
        {phase === "sources" ? (
          <div className="px-8 py-7">
            <Brain className="mx-auto size-10 text-[var(--color-accent)]" />
            <h2 className="mt-3 text-center text-lg font-semibold text-[var(--color-text)]">
              Bienvenue dans Lucid
            </h2>
            <p className="mx-auto mt-1 max-w-md text-center text-sm text-[var(--color-muted)]">
              Ton second cerveau se nourrit de tes sources. Tout est analysé
              <strong> 100 % en local</strong> — rien ne quitte cette machine.
            </p>

            <div className="mt-5 space-y-1.5">
              {connectors.map((c) => (
                <div key={c.id} className="flex items-center gap-3 rounded-xl border border-[var(--color-border)] px-3.5 py-2.5">
                  <ConnectorLogo id={c.id} />
                  <div className="min-w-0 flex-1">
                    <p className="text-sm text-[var(--color-text)]">{c.name}</p>
                    <p className="text-[11px] text-[var(--color-muted)]">
                      {c.connected
                        ? c.conversation_count > 0
                          ? `✓ Détecté — ${c.conversation_count} élément${c.conversation_count > 1 ? "s" : ""}`
                          : "✓ Connecté"
                        : "Non connecté"}
                    </p>
                  </div>
                  {!c.connected && (
                    <button
                      onClick={onOpenSettings}
                      className="shrink-0 rounded-lg border border-[var(--color-border)] px-2.5 py-1 text-xs text-[var(--color-text)] hover:bg-[var(--color-surface-2)]"
                    >
                      Configurer
                    </button>
                  )}
                </div>
              ))}
            </div>

            <div className="mt-6 flex flex-col items-center gap-2">
              <button
                onClick={onGenerate}
                disabled={!hasSource}
                className="flex items-center gap-2 rounded-xl bg-[var(--color-accent)] px-5 py-2.5 text-sm font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-40"
              >
                <Sparkles className="size-4" />
                Générer mon cerveau
              </button>
              {!hasSource && (
                <p className="text-[11px] text-[var(--color-muted)]">
                  Connecte au moins une source pour commencer.
                </p>
              )}
              <button onClick={onSeedDemo} className="mt-1 text-xs text-[var(--color-accent)] hover:underline">
                Explorer une démo (sans connecteur)
              </button>
              <button onClick={onDone} className="text-[11px] text-[var(--color-muted)] hover:text-[var(--color-text)]">
                Plus tard
              </button>
            </div>

            <div className="mt-5"><StepDots current={0} /></div>
          </div>
        ) : (
          <div>
            <div className="px-8 pb-2 pt-7 text-center">
              <span className="text-3xl">🎉</span>
              <h2 className="mt-2 text-lg font-semibold text-[var(--color-text)]">
                Ton cerveau est prêt
              </h2>
              <p className="mx-auto mt-1 max-w-md text-sm text-[var(--color-muted)]">
                Dernière étape : branche tes IA pour qu'elles puissent le consulter.
              </p>
            </div>

            {/* Réutilise la section des Settings telle quelle */}
            <div className="max-h-[320px] overflow-y-auto border-y border-[var(--color-border)]">
              <AiClientsSection />
            </div>

            <div className="space-y-3 px-8 py-5">
              <div className="flex items-center gap-2 rounded-xl border border-[var(--color-border)] bg-[var(--color-surface-2)] px-3 py-2.5">
                <p className="min-w-0 flex-1 truncate font-mono text-[11px] text-[var(--color-muted)]">
                  {EXAMPLE_PROMPT}
                </p>
                <button
                  onClick={copyPrompt}
                  title="Copier le prompt d'exemple"
                  className="flex shrink-0 items-center gap-1 rounded-lg px-2 py-1 text-[11px] text-[var(--color-accent)] hover:bg-[var(--color-accent-soft)]"
                >
                  {copied ? <Check className="size-3" /> : <Copy className="size-3" />}
                  {copied ? "Copié" : "Copier"}
                </button>
              </div>
              <div className="flex items-center justify-between">
                <StepDots current={2} />
                <button
                  onClick={onDone}
                  className="rounded-xl bg-[var(--color-accent)] px-5 py-2 text-sm font-medium text-white hover:opacity-90"
                >
                  C'est parti
                </button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
