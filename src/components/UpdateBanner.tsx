import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

/** Bannière de mise à jour : check silencieux au démarrage, install + relaunch sur clic.
 *  Rien ne s'affiche si l'app est à jour ou si l'endpoint est injoignable (offline). */
export function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [state, setState] = useState<"idle" | "installing" | "error">("idle");

  useEffect(() => {
    // ponytail: un seul check au démarrage ; ajouter un recheck périodique si les sessions durent des jours.
    check().then((u) => { if (u) setUpdate(u); }).catch(() => {});
  }, []);

  if (!update) return null;

  const install = async () => {
    setState("installing");
    try {
      await update.downloadAndInstall();
      await relaunch();
    } catch {
      setState("error");
    }
  };

  return (
    <div className="panel absolute bottom-6 right-6 z-40 flex items-center gap-3 rounded-2xl px-4 py-2.5 text-sm text-[var(--color-text)]">
      {state === "error" ? (
        <span className="text-[var(--color-err)]">Échec de la mise à jour — réessaie plus tard.</span>
      ) : (
        <>
          <span>Mise à jour {update.version} disponible</span>
          <button
            onClick={install}
            disabled={state === "installing"}
            className="cursor-pointer rounded-full bg-[var(--color-accent)] px-3 py-1 text-xs font-medium text-white hover:bg-[var(--color-accent-hover)] disabled:opacity-60"
          >
            {state === "installing" ? "Installation…" : "Installer et redémarrer"}
          </button>
        </>
      )}
    </div>
  );
}
