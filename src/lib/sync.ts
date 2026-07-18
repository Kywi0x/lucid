/** Sync cloud du cerveau : le brain suit le compte entre machines.
 *
 *  Modèle v1 (décision Liam, 2026-07-17, boucle continue 2026-07-18) : un fichier
 *  `sync.zip` par user dans le bucket `backups` (~2 Mo), last-write-wins. La
 *  passe `syncNow` tourne au boot, toutes les 15 s, au focus de la fenêtre et à
 *  la fermeture : push si le local a changé (empreinte mtime côté Rust), pull si
 *  le cloud a changé, conflit tranché à la dernière écriture. L'analyse reste
 *  100 % locale — seul le résultat transite, protégé par RLS. `import_backup`
 *  garde une copie locale avant écrasement. L'état de la sync est observable
 *  (`useSyncStatus`) — jamais d'échec silencieux.
 */
import { useSyncExternalStore } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import type { RealtimeChannel } from "@supabase/supabase-js";
import { supabase, BACKUP_BUCKET } from "./supabase";
import { exportBackup, mergeBackup, syncFingerprint } from "./api";

export const SYNC_FILE = "sync.zip";

/** État de la dernière sync réussie sur cette machine (par compte). */
interface Marker {
  remote: string; // updated_at du sync.zip distant
  fingerprint: number; // empreinte locale au moment du push/pull
}

const markerKey = (uid: string) => `lucid.sync.${uid}`;
function readMarker(uid: string): Marker | null {
  try { return JSON.parse(localStorage.getItem(markerKey(uid)) ?? "null"); } catch { return null; }
}
function writeMarker(uid: string, m: Marker) {
  localStorage.setItem(markerKey(uid), JSON.stringify(m));
}

async function uid(): Promise<string | null> {
  if (!supabase) return null;
  const { data } = await supabase.auth.getSession();
  return data.session?.user.id ?? null;
}

async function remoteUpdatedAt(id: string): Promise<string | null> {
  const { data, error } = await supabase!.storage.from(BACKUP_BUCKET).list(id, { search: SYNC_FILE });
  if (error) throw new Error(`cloud injoignable : ${error.message}`);
  const f = data?.find((f) => f.name === SYNC_FILE);
  return f?.updated_at ?? f?.created_at ?? null;
}

// ── État observable (affiché dans Réglages → Compte) ────────────────────────
export interface SyncState {
  phase: "idle" | "syncing" | "ok" | "error";
  at: number | null; // timestamp de la dernière passe réussie
  detail?: string; // message d'erreur humain
}
let state: SyncState = { phase: "idle", at: null };
const listeners = new Set<() => void>();
function setSyncState(patch: Partial<SyncState>) {
  state = { ...state, ...patch };
  listeners.forEach((l) => l());
}
export function useSyncStatus(): SyncState {
  return useSyncExternalStore(
    (cb) => { listeners.add(cb); return () => listeners.delete(cb); },
    () => state,
  );
}

async function push(id: string): Promise<void> {
  const bytes = await exportBackup();
  const { error } = await supabase!.storage.from(BACKUP_BUCKET).upload(
    `${id}/${SYNC_FILE}`,
    new Blob([bytes.buffer as ArrayBuffer], { type: "application/zip" }),
    { upsert: true },
  );
  if (error) throw new Error(`envoi impossible : ${error.message}`);
  writeMarker(id, { remote: (await remoteUpdatedAt(id)) ?? "", fingerprint: await syncFingerprint() });
  // Préviens les autres machines connectées : elles tirent tout de suite au
  // lieu d'attendre leur tick (simple ping, aucune donnée ne transite ici).
  void channel?.send({ type: "broadcast", event: "pushed", payload: {} });
}

/** Tire le cloud et le FUSIONNE dans le local (nœud le plus récent gagne, rien
 *  n'est perdu). Renvoie true si le local avait des choses que le cloud n'a pas
 *  → l'appelant doit repousser. */
async function pull(id: string, remote: string): Promise<boolean> {
  const { data, error } = await supabase!.storage.from(BACKUP_BUCKET).download(`${id}/${SYNC_FILE}`);
  if (error || !data) throw new Error(`téléchargement impossible : ${error?.message ?? "vide"}`);
  const report = await mergeBackup(new Uint8Array(await data.arrayBuffer()));
  // De vraies données sont arrivées : l'état démo ne s'applique plus.
  localStorage.removeItem("lucid.demo");
  writeMarker(id, { remote, fingerprint: await syncFingerprint() });
  return report.local_extra;
}

let running = false;
let notifyPulled: (() => void) | null = null;
let channel: RealtimeChannel | null = null;

/** Une passe de sync complète : push si le local a changé, pull si le cloud a
 *  changé, conflit tranché à la dernière écriture. Sans effet si déconnecté. */
export async function syncNow(): Promise<void> {
  const id = await uid();
  if (!id || running) return;
  running = true;
  setSyncState({ phase: "syncing" });
  try {
    const fp = await syncFingerprint();
    const marker = readMarker(id);
    const remote = await remoteUpdatedAt(id);
    const localDirty = fp !== 0 && fp !== marker?.fingerprint;
    // fp === 0 : dossier vide OU contenu d'exemple (fingerprint côté Rust
    // renvoie 0 si demo.flag) → le cloud fait foi, la démo n'est jamais poussée.
    const remoteNew = remote !== null && (fp === 0 || remote !== marker?.remote);
    let mustPush = localDirty;
    if (remoteNew) {
      // Fusion (pas de remplacement) : les deux côtés peuvent avoir bougé,
      // le nœud le plus récent gagne, rien n'est perdu — puis on repousse
      // l'union si le local avait des choses que le cloud n'a pas.
      mustPush = await pull(id, remote!);
      notifyPulled?.();
    }
    if (mustPush) await push(id);
    setSyncState({ phase: "ok", at: Date.now(), detail: undefined });
  } catch (e) {
    console.warn("sync:", e);
    setSyncState({ phase: "error", detail: String((e as Error)?.message ?? e) });
  } finally {
    running = false;
  }
}

/** Démarre la sync continue, événementielle des deux côtés :
 *  - push : le watcher Rust émet `user-data-changed` (débouncé 2 s) à toute
 *    écriture dans le périmètre de backup, d'où qu'elle vienne (UI, MCP, watch) ;
 *  - pull : chaque push annonce un ping Realtime `sync-<uid>` → les autres
 *    machines tirent en ~1 s ;
 *  - filets : passe initiale au boot (gate du seed démo via `onChecked`), tick
 *    60 s (offline, ping raté), focus fenêtre, et fermeture de l'app (retenue
 *    4 s max, le temps de pousser la dernière modif).
 *  `onPulled` est appelé à CHAQUE pull qui remplace le local — l'app y recharge
 *  graphe/espaces. Renvoie le stop (cleanup React). */
export function startAutoSync(onPulled: () => void, onChecked?: () => void): () => void {
  notifyPulled = onPulled;
  let stopped = false;
  syncNow().finally(() => { if (!stopped) onChecked?.(); });
  // Ping des autres machines (broadcast, pas de données) → pull immédiat.
  uid().then((id) => {
    if (!id || stopped || !supabase) return;
    channel = supabase
      .channel(`sync-${id}`, { config: { broadcast: { self: false } } })
      .on("broadcast", { event: "pushed" }, () => void syncNow())
      .subscribe();
  });
  // Écritures locales (watcher Rust, déjà débouncé) → push immédiat.
  const unlistenData = listen("user-data-changed", () => void syncNow());
  // ponytail: tick 60 s conservé en filet (retour de offline, ping Realtime
  // manqué) — les événements font le chemin rapide.
  const timer = setInterval(() => void syncNow(), 60_000);
  const onFocus = () => void syncNow();
  window.addEventListener("focus", onFocus);
  let closing = false;
  const win = getCurrentWindow();
  const unlistenClose = win.onCloseRequested(async (e) => {
    if (closing) return;
    closing = true;
    e.preventDefault();
    await Promise.race([syncNow(), new Promise((r) => setTimeout(r, 4000))]);
    void win.destroy();
  });
  return () => {
    stopped = true;
    if (notifyPulled === onPulled) notifyPulled = null; // StrictMode : ne pas écraser le mount suivant
    clearInterval(timer);
    window.removeEventListener("focus", onFocus);
    unlistenClose.then((fn) => fn());
    unlistenData.then((fn) => fn());
    void channel?.unsubscribe();
    channel = null;
  };
}
