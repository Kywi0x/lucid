import { useState } from "react";
import { Check, Copy, Sparkles } from "lucide-react";
import type { ConnectorStatus } from "@/lib/types";
import { ConnectorLogo, AiClientsSection } from "@/components/SettingsModal";
import { NeuralBg, LucidOrb } from "@/components/NeuralBg";
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
            "rounded-full transition-all",
            i === current ? "h-1.5 w-4 bg-[var(--color-accent)]" : "size-1.5",
            i < current && "bg-[var(--color-accent)]/50",
            i > current && "bg-[var(--color-border)]",
          )}
        />
      ))}
    </div>
  );
}

/** Point d'état façon HUD : pastille + label mono, jamais d'aplat de couleur. */
function ConnectorState({ c }: { c: ConnectorStatus }) {
  return (
    <span className="flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-[0.08em]">
      <span
        className={cn(
          "size-1.5 rounded-full",
          c.connected
            ? "bg-[var(--color-ok)] shadow-[0_0_6px_color-mix(in_srgb,var(--color-ok)_60%,transparent)]"
            : "border border-[var(--color-muted)]/50",
        )}
      />
      <span className="text-[var(--color-muted)]">
        {c.connected
          ? c.conversation_count > 0
            ? `${c.conversation_count} élément${c.conversation_count > 1 ? "s" : ""}`
            : "connecté"
          : "off"}
      </span>
    </span>
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
    <div className="absolute inset-0 z-40 flex items-center justify-center">
      <NeuralBg />
      <div className="lucid-rise panel relative w-[560px] max-w-[calc(100vw-2rem)] overflow-hidden rounded-2xl">
        {phase === "sources" ? (
          <div className="px-8 py-7">
            <div className="flex flex-col items-center text-center">
              <LucidOrb size={40} />
              <p className="mt-3 font-mono text-[10px] uppercase tracking-[0.16em] text-[var(--color-muted)]">
                Bienvenue · étape 1/3
              </p>
              <h2 className="mt-1.5 text-lg font-semibold tracking-tight text-[var(--color-text)]">
                Nourris ton second cerveau
              </h2>
              <p className="mx-auto mt-1 max-w-md text-sm leading-relaxed text-[var(--color-muted)]">
                Lucid se construit à partir de tes sources, analysées
                <strong className="text-[var(--color-text)]"> 100 % en local</strong> —
                rien ne quitte cette machine.
              </p>
            </div>

            <div className="mt-5 overflow-hidden rounded-xl border border-[var(--color-border)]">
              {connectors.map((c, i) => (
                <div
                  key={c.id}
                  className={cn(
                    "flex items-center gap-3 px-3.5 py-2.5",
                    i > 0 && "border-t border-[var(--color-border)]",
                  )}
                >
                  <ConnectorLogo id={c.id} />
                  <p className="min-w-0 flex-1 truncate text-sm text-[var(--color-text)]">{c.name}</p>
                  <ConnectorState c={c} />
                  {!c.connected && (
                    <button
                      onClick={onOpenSettings}
                      className="shrink-0 rounded-lg border border-[var(--color-border)] px-2.5 py-1 text-xs text-[var(--color-text)] transition-colors hover:border-[var(--color-accent)]/40 hover:text-[var(--color-accent)]"
                    >
                      Connecter
                    </button>
                  )}
                </div>
              ))}
            </div>

            <div className="mt-6 flex flex-col items-center gap-2">
              <button
                onClick={onGenerate}
                disabled={!hasSource}
                className="flex items-center gap-2 rounded-xl bg-[var(--color-accent)] px-5 py-2.5 text-sm font-medium text-white transition-all hover:bg-[var(--color-accent-hover)] hover:shadow-[0_4px_20px_color-mix(in_srgb,var(--color-accent)_40%,transparent)] disabled:opacity-40"
              >
                <Sparkles className="size-4" />
                Générer mon cerveau
              </button>
              {!hasSource && (
                <p className="font-mono text-[10px] uppercase tracking-[0.08em] text-[var(--color-muted)]">
                  connecte au moins une source
                </p>
              )}
              <button onClick={onSeedDemo} className="mt-1 text-xs text-[var(--color-accent)] hover:underline">
                Explorer une démo (sans connecteur)
              </button>
              <button onClick={onDone} className="text-[11px] text-[var(--color-muted)] transition-colors hover:text-[var(--color-text)]">
                Plus tard
              </button>
            </div>

            <div className="mt-5"><StepDots current={0} /></div>
          </div>
        ) : (
          <div>
            <div className="px-8 pb-2 pt-7 text-center">
              <div className="flex justify-center"><LucidOrb size={40} /></div>
              <p className="mt-3 font-mono text-[10px] uppercase tracking-[0.16em] text-[var(--color-muted)]">
                étape 3/3
              </p>
              <h2 className="mt-1.5 text-lg font-semibold tracking-tight text-[var(--color-text)]">
                Ton cerveau est prêt
              </h2>
              <p className="mx-auto mt-1 max-w-md text-sm leading-relaxed text-[var(--color-muted)]">
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
                  className="rounded-xl bg-[var(--color-accent)] px-5 py-2 text-sm font-medium text-white transition-all hover:bg-[var(--color-accent-hover)] hover:shadow-[0_4px_20px_color-mix(in_srgb,var(--color-accent)_40%,transparent)]"
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
