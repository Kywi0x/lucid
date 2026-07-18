import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** Chip « BETA » affiché si la version installée est une pré-release (contient
 *  un tiret, ex. 0.2.0-beta.3). Déduit du numéro de version : aucune config,
 *  impossible d'être faux. Change aussi le titre de la fenêtre en « Lucid Beta »
 *  pour distinguer l'app de test au premier coup d'œil (dock, barre des tâches). */
export function BetaBadge() {
  const [beta, setBeta] = useState(false);
  useEffect(() => {
    getVersion()
      .then((v) => {
        if (v.includes("-")) {
          setBeta(true);
          void getCurrentWindow().setTitle("Lucid Beta");
        }
      })
      .catch(() => {});
  }, []);
  if (!beta) return null;
  return (
    <span className="mx-1 rounded-full bg-[var(--color-accent-soft)] px-2 py-0.5 font-mono text-[10px] font-semibold uppercase tracking-wider text-[var(--color-accent)]">
      beta
    </span>
  );
}
