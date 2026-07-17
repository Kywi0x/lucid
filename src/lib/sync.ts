/** Sync cloud du cerveau : le brain suit le compte entre machines.
 *
 *  Modèle v1 (décision Liam, 2026-07-17) : un fichier `sync.zip` par user dans
 *  le bucket `backups` (même périmètre que la sauvegarde manuelle, ~2 Mo),
 *  last-write-wins. Pull au démarrage si le cloud a changé depuis la dernière
 *  sync de CETTE machine ; push quand les données locales changent (empreinte
 *  mtime côté Rust). L'analyse reste 100 % locale — seul le résultat transite,
 *  protégé par RLS. `import_backup` garde une copie locale avant écrasement.
 */
import { supabase, BACKUP_BUCKET } from "./supabase";
import { exportBackup, importBackup, syncFingerprint } from "./api";

const FILE = "sync.zip";

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
  const { data } = await supabase!.storage.from(BACKUP_BUCKET).list(id, { search: FILE });
  const f = data?.find((f) => f.name === FILE);
  return f?.updated_at ?? f?.created_at ?? null;
}

let pushing = false;

/** Push si les données locales ont changé depuis la dernière sync. Silencieux
 *  en cas d'échec (offline, policy manquante) — on retentera au tick suivant. */
export async function pushIfDirty(): Promise<void> {
  const id = await uid();
  if (!id || pushing) return;
  const fp = await syncFingerprint();
  if (fp === 0 || fp === readMarker(id)?.fingerprint) return;
  pushing = true;
  try {
    const bytes = await exportBackup();
    const { error } = await supabase!.storage.from(BACKUP_BUCKET).upload(
      `${id}/${FILE}`,
      new Blob([bytes.buffer as ArrayBuffer], { type: "application/zip" }),
      { upsert: true },
    );
    if (error) throw error;
    writeMarker(id, { remote: (await remoteUpdatedAt(id)) ?? "", fingerprint: fp });
  } catch (e) {
    console.warn("sync push:", e);
  } finally {
    pushing = false;
  }
}

/** Pull au démarrage si le cloud a bougé depuis la dernière sync d'ici.
 *  Renvoie true si le cerveau local a été remplacé (recharger le graphe). */
export async function pullIfNewer(): Promise<boolean> {
  const id = await uid();
  if (!id) return false;
  try {
    const remote = await remoteUpdatedAt(id);
    if (!remote) return false; // rien dans le cloud → le prochain push seedera
    // « Déjà à jour » ne vaut que si des données locales existent : si le dossier
    // user a été vidé (reset, réinstall), le cloud fait foi et on restaure —
    // sinon le marqueur périmé bloquerait le pull et le push écraserait le cloud.
    const fp = await syncFingerprint();
    if (fp !== 0 && readMarker(id)?.remote === remote) return false;
    const { data, error } = await supabase!.storage.from(BACKUP_BUCKET).download(`${id}/${FILE}`);
    if (error || !data) throw error ?? new Error("téléchargement vide");
    await importBackup(new Uint8Array(await data.arrayBuffer()));
    writeMarker(id, { remote, fingerprint: await syncFingerprint() });
    return true;
  } catch (e) {
    console.warn("sync pull:", e);
    return false;
  }
}

/** Démarre la sync auto : pull initial, puis push périodique tant que l'app
 *  est ouverte. `onChecked` est appelé après le pull initial (réussi ou non) —
 *  l'app attend ce signal avant de seeder le starter démo, sinon la démo
 *  gagnerait la course contre la restauration cloud. Renvoie le stop (cleanup React). */
export function startAutoSync(onPulled: () => void, onChecked?: () => void): () => void {
  let stopped = false;
  pullIfNewer().then((pulled) => {
    if (stopped) return;
    if (pulled) onPulled();
    onChecked?.();
    void pushIfDirty(); // seed du cloud / rattrapage de la session précédente
  });
  const timer = setInterval(() => void pushIfDirty(), 60_000);
  return () => { stopped = true; clearInterval(timer); };
}
