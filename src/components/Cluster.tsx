import { useEffect, useRef, useState } from "react";
import { useTheme, type ThemeMode } from "@/lib/theme";

const NEXT: Record<ThemeMode, ThemeMode> = { system: "light", light: "dark", dark: "system" };
const THEME_LABEL: Record<ThemeMode, string> = {
  system: "Thème : automatique",
  light: "Thème : clair",
  dark: "Thème : sombre",
};

/** Cluster haut-droite : notifications · réglages · thème (maquette « l'app
 *  vivante »). Le compteur porte ce qui attend une DÉCISION ; le reste est un
 *  compte rendu, il mérite un point, pas un nombre qui réclame qu'on le vide. */
export function Cluster({
  pending, hasNews, notifOpen, onToggleNotif, onOpenSettings, extra,
}: {
  pending: number;
  hasNews: boolean;
  notifOpen: boolean;
  onToggleNotif: () => void;
  onOpenSettings: () => void;
  /** Boutons propres à l'app (régénérer, archiviste, historique) — la maquette
   *  n'en a pas, mais ces gestes existent et personne n'a demandé de les
   *  retirer. Même matière, même pastille, posés avant les notifications. */
  extra?: React.ReactNode;
}) {
  const { mode, setMode } = useTheme();
  const [bump, setBump] = useState(false);
  const prev = useRef(pending);

  // Le badge tressaille quand une décision de plus arrive — sinon un nombre
  // qui change tout seul dans un coin ne se voit pas.
  useEffect(() => {
    if (pending > prev.current) {
      setBump(true);
      const t = setTimeout(() => setBump(false), 640);
      prev.current = pending;
      return () => clearTimeout(t);
    }
    prev.current = pending;
  }, [pending]);

  return (
    <div className="cluster lg">
      {extra}
      <button
        id="ncBtn"
        className={"badge" + (bump ? " bump" : "")}
        title="Notifications"
        aria-label="Notifications"
        aria-expanded={notifOpen}
        data-n={pending}
        data-news={hasNews}
        onClick={onToggleNotif}
      >
        ✦<span className="cnt">{pending}</span>
      </button>
      <button title="Réglages (⌘,)" aria-label="Réglages" onClick={onOpenSettings}>⚙</button>
      <button
        title={THEME_LABEL[mode]}
        aria-label="Changer de thème"
        onClick={() => setMode(NEXT[mode])}
      >
        ◐
      </button>
    </div>
  );
}
