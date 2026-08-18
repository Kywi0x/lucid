import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Loader2 } from "lucide-react";

/** Nom court par connecteur — la barre d'outils n'a pas la place d'un libellé long. */
const LABELS: Record<string, string> = {
  "google-drive": "Drive",
  "apple-notes": "Notes",
  "local-folder": "Dossiers",
};

/**
 * Témoin « une source est en train de se synchroniser », dans la barre d'outils.
 *
 * Le travail tournait déjà en fond — c'est l'affichage qui manquait : le statut
 * vivait dans la modale des réglages, donc la fermer suffisait à ne plus rien
 * voir. Et la synchro **automatique** (watcher toutes les 5 min) n'avait, elle,
 * jamais eu le moindre témoin nulle part.
 */
export function SyncBadge() {
  const [active, setActive] = useState<string[]>([]);

  useEffect(() => {
    const p = listen<{ source: string; active: boolean }>("connector-sync", (e) => {
      const { source, active: on } = e.payload;
      setActive((prev) =>
        on ? (prev.includes(source) ? prev : [...prev, source]) : prev.filter((s) => s !== source),
      );
    });
    return () => { void p.then((un) => un()); };
  }, []);

  if (active.length === 0) return null;

  const names = active.map((s) => LABELS[s] ?? s).join(", ");
  return (
    <span
      title={`Synchronisation en cours : ${names}`}
      className="flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[10px] text-[var(--color-muted)]"
    >
      <Loader2 className="size-3 animate-spin" />
      <span className="max-w-[9rem] truncate">{names}</span>
    </span>
  );
}
