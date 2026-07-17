import { useState, type ReactNode } from "react";
import { Check, ChevronDown, Copy } from "lucide-react";
import { cn, copyText } from "@/lib/utils";

/** Mini-tutos « brancher son IA » — étapes vérifiées en réel le 2026-07-16.
 *  Ces interfaces bougent : si un menu a changé, c'est ici qu'on met à jour. */

const TABS = ["claude.ai", "ChatGPT", "Claude Code"] as const;
type Tab = (typeof TABS)[number];

function Step({ n, children }: { n: number; children: ReactNode }) {
  return (
    <li className="flex items-start gap-2.5">
      <span className="mt-0.5 flex size-4 shrink-0 items-center justify-center rounded border border-[var(--color-accent)]/35 font-mono text-[9px] text-[var(--color-accent)]">
        {n}
      </span>
      <span className="min-w-0 text-[11px] leading-relaxed text-[var(--color-text)]">{children}</span>
    </li>
  );
}

function Menu({ children }: { children: ReactNode }) {
  return <code className="rounded bg-[var(--color-surface-2)] px-1 py-0.5 font-mono text-[10px] text-[var(--color-text)]">{children}</code>;
}

function Warn({ children }: { children: ReactNode }) {
  return (
    <p className="flex items-start gap-1.5 text-[10.5px] leading-relaxed text-[var(--color-warn)]">
      <span className="mt-px shrink-0">⚠</span>
      <span>{children}</span>
    </p>
  );
}

/** Guide dépliable pour brancher le connecteur MCP à claude.ai / ChatGPT / Claude Code.
 *  `url` absent → placeholders (utilisé depuis Settings, où l'URL vit dans Partager). */
export function McpConnectGuide({ url }: { url?: string | null }) {
  const [open, setOpen] = useState(false);
  const [tab, setTab] = useState<Tab>("claude.ai");
  const [copied, setCopied] = useState(false);

  const u = url ?? "<URL MCP — modale Partager → Connecteur IA>";
  const cmd = `claude mcp add --transport http lucid "${u}"`;

  async function copyCmd() {
    if (await copyText(cmd)) { setCopied(true); setTimeout(() => setCopied(false), 2000); }
  }

  return (
    <div className="mt-2">
      <button
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1 text-[11px] font-medium text-[var(--color-accent)] hover:underline"
      >
        <ChevronDown className={cn("size-3 transition-transform", !open && "-rotate-90")} />
        Comment brancher mon IA ?
      </button>

      {open && (
        <div className="mt-2 overflow-hidden rounded-xl border border-[var(--color-border)]">
          {/* Onglets */}
          <div className="flex gap-1 border-b border-[var(--color-border)] bg-[var(--color-surface-2)]/60 p-1">
            {TABS.map((t) => (
              <button
                key={t}
                onClick={() => setTab(t)}
                className={cn(
                  "rounded-lg px-2.5 py-1 text-[11px] font-medium transition-colors",
                  tab === t
                    ? "bg-[var(--color-surface)] text-[var(--color-text)] shadow-sm"
                    : "text-[var(--color-muted)] hover:text-[var(--color-text)]",
                )}
              >
                {t}
              </button>
            ))}
          </div>

          <div className="space-y-2.5 px-3.5 py-3">
            {tab === "claude.ai" && (
              <>
                <ol className="space-y-2">
                  <Step n={1}>Sur <Menu>claude.ai</Menu> (web) : tes initiales en bas à gauche → <Menu>Paramètres</Menu> → <Menu>Connecteurs</Menu>.</Step>
                  <Step n={2}>En bas : <Menu>Ajouter un connecteur personnalisé</Menu>.</Step>
                  <Step n={3}>Colle l'URL du connecteur — laisse les champs OAuth vides — puis <Menu>Ajouter</Menu>.</Step>
                  <Step n={4}>Dans une conversation : bouton outils → active <strong>Lucid</strong>. Fonctionne aussi sur mobile une fois ajouté.</Step>
                </ol>
                <Warn>Workspace Team/Entreprise : seul le <strong>propriétaire</strong> du workspace voit ce bouton.</Warn>
              </>
            )}

            {tab === "ChatGPT" && (
              <>
                <ol className="space-y-2">
                  <Step n={1}>Sur <Menu>chatgpt.com</Menu> (web) : avatar → <Menu>Settings</Menu> → <Menu>Connectors</Menu> → <Menu>Advanced settings</Menu> → active <strong>Developer mode</strong>.</Step>
                  <Step n={2}>Retour dans <Menu>Connectors</Menu> → <Menu>Créer</Menu> : nom <Menu>Lucid</Menu>, colle l'URL du connecteur.</Step>
                  <Step n={3}>Authentification : <Menu>Aucune authentification</Menu>, coche « J'ai compris », puis <Menu>Créer</Menu>. L'avertissement rouge d'OpenAI est normal (serveur non vérifié par eux — c'est le tien).</Step>
                  <Step n={4}>Dans une conversation : ＋ / Outils → <Menu>Developer mode</Menu> → active <strong>Lucid</strong>.</Step>
                </ol>
                <Warn>Nécessite un plan Plus/Pro (web uniquement, pas l'app mobile).</Warn>
              </>
            )}

            {tab === "Claude Code" && (
              <>
                <ol className="space-y-2">
                  <Step n={1}>Dans un terminal, une seule commande :</Step>
                </ol>
                <div className="flex items-center gap-2 rounded-lg bg-[var(--color-surface-2)] px-2.5 py-2">
                  <code className="min-w-0 flex-1 truncate font-mono text-[10px] text-[var(--color-text)]">{cmd}</code>
                  <button onClick={copyCmd} title="Copier la commande"
                    className="shrink-0 rounded p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface)] hover:text-[var(--color-text)]">
                    {copied ? <Check className="size-3.5 text-[var(--color-ok)]" /> : <Copy className="size-3.5" />}
                  </button>
                </div>
                <ol className="space-y-2">
                  <Step n={2}>Lance <Menu>claude</Menu> — les outils <Menu>brain_*</Menu> sont disponibles dans la session.</Step>
                </ol>
              </>
            )}

            <p className="border-t border-[var(--color-border)] pt-2 text-[10.5px] leading-relaxed text-[var(--color-muted)]">
              <strong className="text-[var(--color-text)]">Test de réussite</strong> — demande :
              « <em>Fais-moi un tour d'horizon de mon second cerveau</em> ». Si l'IA cite tes
              projets, c'est branché. Elle peut aussi <strong>proposer</strong> des notes :
              elles apparaissent en bulles à valider dans Lucid — rien ne s'écrit sans toi.
            </p>
          </div>
        </div>
      )}
    </div>
  );
}
