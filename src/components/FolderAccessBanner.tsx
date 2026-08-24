import { useState } from "react";
import { FolderLock } from "lucide-react";
import { openPrivacySettings } from "@/lib/api";

/** Un ou plusieurs dossiers du scan ont été **refusés par l'OS**.
 *
 *  Avant, ce cas était indiscernable d'un dossier vide : `read_dir` échouait,
 *  le parcours rendait la main, et le scan se terminait « avec succès » sur zéro
 *  fichier. Sur un Mac neuf c'est le premier écran d'un testeur — il en conclut
 *  que Lucid ne trouve rien, et il a raison de le croire.
 *
 *  macOS met Bureau/Documents/Téléchargements derrière un consentement par app
 *  (TCC) ; un refus tient jusqu'aux Réglages Système, d'où le bouton. Ailleurs,
 *  un refus vient des droits du dossier : on l'explique, on ne feint pas un
 *  panneau qui n'existe pas (ADR-0015). */
export function FolderAccessBanner({ folders, onDismiss }: { folders: string[]; onDismiss: () => void }) {
  const [error, setError] = useState("");
  const isMac = navigator.userAgent.includes("Mac");
  const name = (p: string) => p.split(/[/\\]/).filter(Boolean).pop() || p;

  return (
    <div className="panel absolute bottom-6 right-6 z-40 flex max-w-sm flex-col gap-2 rounded-2xl px-4 py-3 text-sm text-[var(--color-text)]">
      <p className="flex items-center gap-2 font-medium">
        <FolderLock className="size-4 shrink-0 text-[var(--color-err)]" />
        {folders.length > 1
          ? `${folders.length} dossiers n'ont pas pu être lus`
          : `Lucid n'a pas pu lire « ${name(folders[0])} »`}
      </p>
      <p className="text-xs leading-relaxed text-[var(--color-muted)]">
        {isMac
          ? "macOS bloque l'accès tant que l'autorisation n'est pas donnée. Leur contenu est absent de ton cerveau — pas vide, invisible."
          : "L'accès a été refusé par le système. Leur contenu est absent de ton cerveau — pas vide, invisible."}
      </p>
      <ul className="max-h-20 overflow-y-auto text-[11px] text-[var(--color-muted)]">
        {folders.map((f) => <li key={f} className="truncate" title={f}>{f}</li>)}
      </ul>
      {error && <p className="text-[11px] text-[var(--color-err)]">{error}</p>}
      <div className="flex gap-1.5">
        {isMac && (
          <button
            onClick={() => { setError(""); openPrivacySettings().catch((e) => setError(String(e))); }}
            className="cursor-pointer rounded-full bg-[var(--color-accent)] px-3 py-1 text-xs font-medium text-white hover:bg-[var(--color-accent-hover)]"
          >
            Ouvrir les Réglages Système
          </button>
        )}
        <button
          onClick={onDismiss}
          className="cursor-pointer rounded-full px-3 py-1 text-xs font-medium text-[var(--color-muted)] hover:bg-[var(--color-surface-2)]"
        >
          Plus tard
        </button>
      </div>
      <p className="text-[10px] text-[var(--color-muted)]">
        Une fois l'autorisation donnée, relance une synchronisation depuis les Réglages.
      </p>
    </div>
  );
}
