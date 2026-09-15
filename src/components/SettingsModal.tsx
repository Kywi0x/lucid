import { useState, useEffect, useCallback } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { syncNow, useSyncStatus, SYNC_FILE } from "@/lib/sync";
import {
  Loader2,
  Pencil,
  Trash2,
  Plus,
  Check,
} from "lucide-react";
import {
  listModels,
  setActiveModel,
  downloadModel,
  mcpManualValidationEnabled,
  setMcpManualValidation,
  runArchivist,
  exportBackup,
  importBackup,
  resetEnvironment,
  telemetryEnabled,
  setTelemetry,
  sentryActive,
  crashTest,
  archivistDiagnostic,
  aiDiagnostics,
  type AiDiagnostics,
  type ModelInfo,
} from "@/lib/api";
import { supabase, BACKUP_BUCKET } from "@/lib/supabase";
import { notify } from "@/lib/notify";
import type { ConnectorStatus, Space } from "@/lib/types";
import { relativeDate, copyText } from "@/lib/utils";
import { McpConnectGuide } from "@/components/McpConnectGuide";
import { fetchBrainStats, DAYS, type BrainStats } from "@/lib/stats";
import { ensurePersonalMcpUrl, existingPersonalMcpUrl, revokePersonalMcpToken, rotatePersonalMcpUrl } from "@/lib/share";

// Le branchement des sources a quitté les Réglages : il vit dans le Launchpad
// (src/components/Launchpad.tsx), ouvert par le dock. Un seul écran parle des
// connecteurs — deux en parleraient différemment.

// ── Section Spaces (gestion : créer / renommer / supprimer) ──────────────────

function SpacesSection({
  spaces,
  onCreate,
  onRename,
  onDelete,
}: {
  spaces: Space[];
  onCreate: (name: string) => void;
  onRename: (id: string, name: string) => void;
  onDelete: (id: string) => void;
}) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");

  function commitRename() {
    const name = draft.trim();
    if (editingId && name) onRename(editingId, name);
    setEditingId(null);
  }

  function commitCreate() {
    const name = newName.trim();
    if (name) onCreate(name);
    setNewName("");
    setCreating(false);
  }

  return (
    <>
      <div className="grp">
        <div className="sec-top"><span className="h">Espaces</span></div>
        <p className="note">
          Un espace est une vue filtrée du graphe (un projet, des révisions, un client…) —
          partageable en lecture via le bouton Partager de la sidebar.
        </p>
      </div>

      <div className="grp">
        <span className="lbl3">{spaces.length} espace{spaces.length > 1 ? "s" : ""}</span>
        <div className="card">
          {spaces.map((s) => {
            const isLucid = s.id === "lucid";
            return (
              <div className="line group" key={s.id}>
                {editingId === s.id ? (
                  <input
                    autoFocus
                    value={draft}
                    onChange={(e) => setDraft(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitRename();
                      if (e.key === "Escape") { e.stopPropagation(); setEditingId(null); }
                    }}
                    onBlur={commitRename}
                    className="col bg-transparent outline-none"
                  />
                ) : (
                  <span className="col"><span className="t">{s.name}</span></span>
                )}
                <span className="v mut">{s.node_ids ? `${s.node_ids.length} nœuds` : "tout le graphe"}</span>
                {!isLucid && editingId !== s.id && (
                  <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
                    <button
                      className="icon-btn"
                      onClick={() => { setEditingId(s.id); setDraft(s.name); }}
                      title="Renommer"
                    >
                      <Pencil className="size-3" />
                    </button>
                    <button
                      className="icon-btn"
                      onClick={() => onDelete(s.id)}
                      title="Supprimer"
                    >
                      <Trash2 className="size-3" />
                    </button>
                  </span>
                )}
              </div>
            );
          })}
        </div>

        {creating ? (
          <div className="acts">
            <input
              autoFocus
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitCreate();
                if (e.key === "Escape") { e.stopPropagation(); setCreating(false); setNewName(""); }
              }}
              placeholder="Nom de l'espace…"
              className="flex-1 rounded-full bg-[var(--lg-fill)] px-4 py-1.5 text-xs outline-none placeholder:text-[var(--sb-placeholder)] shadow-[inset_0_0_0_0.5px_var(--hairline)]"
            />
            <button className="btn primary" onClick={commitCreate} disabled={!newName.trim()}>Créer</button>
          </div>
        ) : (
          <div className="acts">
            <button className="btn" onClick={() => setCreating(true)}>
              <Plus className="inline size-3" /> Nouvel espace
            </button>
          </div>
        )}
      </div>
    </>
  );
}

// ── Section Modèle IA ─────────────────────────────────────────────────────────

function ModelSection() {
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [dlPercent, setDlPercent] = useState<number | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [diag, setDiag] = useState<AiDiagnostics | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  // L'Archiviste vit ici (maquette « l'app vivante ») : c'est l'IA locale qui
  // range, le réglage appartient à la même page qu'elle.
  const [manualValidation, setManualValidation] = useState(false);
  const [archiving, setArchiving] = useState(false);
  const [archiveResult, setArchiveResult] = useState<string | null>(null);

  const refresh = useCallback(() => listModels().then(setModels), []);
  useEffect(() => { refresh(); }, [refresh]);
  useEffect(() => {
    aiDiagnostics().then(setDiag).catch(() => {});
    mcpManualValidationEnabled().then(setManualValidation).catch(() => {});
  }, []);

  // Sélection d'un modèle : s'il n'est pas téléchargé, on le télécharge DIRECTEMENT
  // (avec barre de progression), sans repasser par le bootstrap — sinon le
  // bootstrap réinstallerait le modèle recommandé et écraserait ce choix.
  async function handlePick(m: ModelInfo) {
    if ((m.active && m.downloaded) || busyId) return;
    setErr(null); setBusyId(m.id);
    try {
      if (!m.downloaded) {
        setDlPercent(0);
        const un = await listen<{ percent: number }>("download-progress", (e) => setDlPercent(e.payload.percent));
        try { await downloadModel(m.id); } finally { un(); setDlPercent(null); }
      } else {
        await setActiveModel(m.id);
      }
      await refresh();
    } catch (e) { setErr(String(e)); }
    finally { setBusyId(null); }
  }

  /** Diagnostic IA : binaires, modèles, débit, logs — copié pour un retour bêta.
   *  Le même rapport que la boucle de test Windows. */
  async function copyDiagnostic() {
    setMsg(null);
    try {
      const d = await aiDiagnostics();
      setDiag(d);
      const yn = (b: boolean) => (b ? "✅" : "❌");
      const report = [
        "=== Diagnostic IA Lucid ===",
        `OS: ${d.os} · RAM: ${d.total_ram_gb.toFixed(1)} Go`,
        `Binaire llama-completion: ${yn(d.completion_binary)}`,
        `Binaire llama-server (serveur + embeddings): ${yn(d.server_binary)}`,
        `Modèle génération: ${d.gen_model ?? "—"} (présent: ${yn(d.gen_model_present)})`,
        `Modèle embedding (BGE-M3): ${yn(d.embed_model_present)}`,
        // Le chiffre qui dit si le GPU travaille. Sans lui, un scan de 85 minutes
        // ressemble à un blocage et personne ne sait pourquoi (run Windows 06/08).
        `Débit génération: ${
          d.gen_tokens_per_second == null
            ? "— (aucune génération depuis le démarrage)"
            : `${d.gen_tokens_per_second.toFixed(1)} tok/s${
                d.gen_tokens_per_second < 20 ? " ⚠️ CPU pur probable — scan très lent" : ""
              }`
        }`,
        "--- llama-server (génération) stderr ---",
        d.gen_server_log || "(vide — serveur jamais démarré)",
        "--- llama-server (embedding) stderr ---",
        d.embed_server_log || "(vide — serveur jamais démarré)",
        "--- lucid.log (60 dernières lignes) ---",
        d.log_tail || "(log vide)",
      ].join("\n");
      setMsg(await copyText(report) ? "Diagnostic IA copié ✓ — colle-le dans ton message." : "Copie impossible.");
    } catch (e) { setMsg(String((e as Error).message ?? e)); }
  }

  async function handleRunArchivist() {
    setArchiving(true);
    setArchiveResult(null);
    try {
      const report = await runArchivist();
      setArchiveResult(report.trim()); // toujours non-vide — le « rien à faire » est explicite côté Rust
    } catch (e) {
      setArchiveResult(String(e instanceof Error ? e.message : e));
    } finally { setArchiving(false); }
  }

  return (
    <>
      <div className="grp">
        <div className="sec-top">
          <span className="h">IA locale</span>
          <button className="btn" onClick={copyDiagnostic}>Diagnostic</button>
        </div>
        <p className="note">Le modèle tourne sur ta machine. Aucun document ne part vers un serveur.</p>
        {msg && <p className="note">{msg}</p>}
      </div>

      <div className="grp">
        <span className="lbl3">Modèle</span>
        <div className="card">
          {models.map((m) => {
            const isBusy = busyId === m.id;
            return (
              <div className="line" key={m.id}>
                <span className="col">
                  <span className="t">{m.name}</span>
                  <span className="s">
                    {m.size_gb} Go · {m.downloaded ? "téléchargé" : "à télécharger"}
                    {m.recommended ? " · recommandé" : ""}
                  </span>
                </span>
                <span className="v">
                  {m.active && m.downloaded ? (
                    <span className="v mut">actif</span>
                  ) : (
                    <button className="btn" onClick={() => handlePick(m)} disabled={busyId !== null}>
                      {isBusy
                        ? (dlPercent !== null ? `${dlPercent} %` : <Loader2 className="inline size-3 animate-spin" />)
                        : m.downloaded ? "Activer" : "Télécharger"}
                    </button>
                  )}
                </span>
              </div>
            );
          })}
          <div className="line">
            <span className="k">Débit mesuré</span>
            <span className="v">
              {diag?.gen_tokens_per_second == null
                ? <span className="v mut">— (aucune génération depuis le démarrage)</span>
                : <>{diag.gen_tokens_per_second.toFixed(0)} tok/s <span className="v mut">({diag.os})</span></>}
            </span>
          </div>
          <div className="line">
            <span className="k">Embeddings</span>
            <span className="v">BGE-M3 · <span className="v mut">{diag?.embed_model_present ? "présent" : "absent"}</span></span>
          </div>
        </div>
        {err && <p className="note warn">{err}</p>}
      </div>

      <div className="grp">
        <span className="lbl3">Archiviste</span>
        <div className="card">
          <div className="line">
            <span className="col">
              <span className="t">Valider manuellement les actions de l'Archiviste</span>
              <span className="s">
                Par défaut il écrit directement dans ton cerveau. Activé, chaque rangement
                attend ta décision dans « À traiter ». Dans les deux cas un snapshot est pris
                avant chaque action, annulable depuis l'Historique.
              </span>
            </span>
            <button
              className="tog"
              aria-pressed={manualValidation}
              aria-label="Valider manuellement les actions de l'Archiviste"
              onClick={async () => {
                const on = !manualValidation;
                setManualValidation(on);
                try { await setMcpManualValidation(on); } catch { setManualValidation(!on); }
              }}
            >
              <i />
            </button>
          </div>
        </div>
        <div className="acts">
          <button className="btn" onClick={handleRunArchivist} disabled={archiving}>
            {archiving ? <Loader2 className="inline size-3 animate-spin" /> : null} Lancer l'Archiviste maintenant
          </button>
        </div>
        {archiveResult && <p className="note" style={{ whiteSpace: "pre-line" }}>{archiveResult}</p>}
      </div>

      <div className="grp">
        <span className="lbl3">Binaires embarqués</span>
        <div className="pills">
          {[
            ["llama-completion", diag?.completion_binary],
            ["llama-server", diag?.server_binary],
          ].map(([name, ok]) => (
            <span className="pill-ok" key={String(name)}>
              <span className="s" style={{ background: ok ? "var(--color-ok)" : "var(--color-err)" }} />
              {String(name)}
            </span>
          ))}
        </div>
        <p className="note">
          Deux binaires vérifiés au lancement. Les outils PDF (pdftotext, pdftoppm, tesseract)
          ne sont pas sondés ici : leur absence est rattrapée par le repli pur Rust, et le
          rapport de scan le dit — jamais d'échec silencieux.
        </p>
      </div>
    </>
  );
}

// ── SettingsModal ─────────────────────────────────────────────────────────────

type Section = "account" | "model" | "mcp" | "spaces";

// ── Section « Compte » : auth Supabase + sauvegarde cloud du cerveau ─────────
function AccountSection({ onRestored }: { onRestored?: () => void }) {
  const sync = useSyncStatus();
  const [session, setSession] = useState<import("@supabase/supabase-js").Session | null>(null);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [backups, setBackups] = useState<{ name: string; created_at: string; size: number }[]>([]);
  const [telemetry, setTelemetryState] = useState(false);
  const [sentryOn, setSentryOn] = useState(false);

  useEffect(() => {
    telemetryEnabled().then(setTelemetryState).catch(() => {});
    sentryActive().then(setSentryOn).catch(() => {});
    if (!supabase) return;
    supabase.auth.getSession().then(({ data }) => setSession(data.session));
    const { data: sub } = supabase.auth.onAuthStateChange((_e, s) => setSession(s));
    return () => sub.subscription.unsubscribe();
  }, []);

  const uid = session?.user.id;

  const refreshBackups = useCallback(async () => {
    if (!supabase || !uid) return;
    const { data, error } = await supabase.storage.from(BACKUP_BUCKET)
      .list(uid, { sortBy: { column: "created_at", order: "desc" }, limit: 20 });
    if (!error && data) {
      // sync.zip = la sync automatique, affichée dans son propre bloc — seules
      // les sauvegardes manuelles datées vont dans la liste « Restaurer ».
      setBackups(data.filter((f) => f.name !== SYNC_FILE).map((f) => ({
        name: f.name,
        created_at: f.created_at ?? "",
        size: (f.metadata as { size?: number } | null)?.size ?? 0,
      })));
    }
  }, [uid]);
  useEffect(() => { refreshBackups(); }, [refreshBackups]);

  async function auth(mode: "signin" | "signup") {
    if (!supabase || !email.trim() || password.length < 6) {
      setMsg("Email + mot de passe (6 caractères min).");
      return;
    }
    setBusy(mode); setMsg(null);
    try {
      const { error, data } = mode === "signin"
        ? await supabase.auth.signInWithPassword({ email: email.trim(), password })
        : await supabase.auth.signUp({ email: email.trim(), password });
      if (error) throw error;
      if (mode === "signup" && !data.session) {
        setMsg("Compte créé — vérifie ta boîte mail pour confirmer, puis connecte-toi.");
      }
    } catch (e) {
      setMsg(String((e as Error).message ?? e));
    } finally { setBusy(null); }
  }

  async function handleBackup() {
    if (!supabase || !uid) return;
    setBusy("backup"); setMsg(null);
    try {
      const bytes = await exportBackup();
      const name = `${new Date().toISOString().replace(/[:.]/g, "-")}.zip`;
      const { error } = await supabase.storage.from(BACKUP_BUCKET)
        .upload(`${uid}/${name}`, new Blob([bytes.buffer as ArrayBuffer], { type: "application/zip" }));
      if (error) throw error;
      setMsg(`Sauvegardé ✓ (${(bytes.length / 1024 / 1024).toFixed(1)} Mo)`);
      await refreshBackups();
    } catch (e) {
      setMsg(String((e as Error).message ?? e));
    } finally { setBusy(null); }
  }

  async function handleRestore(name: string) {
    if (!supabase || !uid) return;
    if (!confirm("Restaurer cette sauvegarde ? Le cerveau actuel sera remplacé (une copie locale est gardée).")) return;
    setBusy(name); setMsg(null);
    try {
      const { data, error } = await supabase.storage.from(BACKUP_BUCKET).download(`${uid}/${name}`);
      if (error || !data) throw error ?? new Error("Téléchargement vide");
      const n = await importBackup(new Uint8Array(await data.arrayBuffer()));
      setMsg(`${n} fichiers restaurés ✓`);
      onRestored?.();
    } catch (e) {
      setMsg(String((e as Error).message ?? e));
    } finally { setBusy(null); }
  }

  if (!supabase) {
    return (
      <div className="grp">
        <div className="sec-top"><span className="h">Compte</span></div>
        <p className="note">
          Supabase n'est pas configuré : remplis <code>VITE_SUPABASE_URL</code> et{" "}
          <code>VITE_SUPABASE_ANON_KEY</code> dans <code>.env</code> puis relance l'app.
        </p>
      </div>
    );
  }

  return (
    <>
      <div className="grp">
        <div className="sec-top"><span className="h">Compte</span></div>
        {session ? (
          <div className="who">
            <span className="av">{(session.user.email ?? "?").slice(0, 1).toUpperCase()}</span>
            <span className="col">
              <span className="em">{session.user.email}</span>
              <span className="pl">Compte Lucid · beta</span>
            </span>
          </div>
        ) : (
          <div className="card">
            <div className="line">
              <span className="k">Email</span>
              <input
                type="email" value={email} onChange={(e) => setEmail(e.target.value)}
                placeholder="email@exemple.fr"
                className="v flex-1 bg-transparent text-right outline-none placeholder:text-[var(--sb-placeholder)]"
              />
            </div>
            <div className="line">
              <span className="k">Mot de passe</span>
              <input
                type="password" value={password} onChange={(e) => setPassword(e.target.value)}
                placeholder="6 caractères min."
                className="v flex-1 bg-transparent text-right outline-none placeholder:text-[var(--sb-placeholder)]"
              />
            </div>
            <div className="line">
              <span className="col"><span className="s">Le compte donne accès à l'app et fait suivre ton cerveau entre tes machines.</span></span>
              <span className="v" style={{ display: "flex", gap: 8 }}>
                <button className="btn" onClick={() => auth("signup")} disabled={busy !== null}>
                  {busy === "signup" ? <Loader2 className="inline size-3 animate-spin" /> : null} Créer un compte
                </button>
                <button className="btn primary" onClick={() => auth("signin")} disabled={busy !== null}>
                  {busy === "signin" ? <Loader2 className="inline size-3 animate-spin" /> : null} Se connecter
                </button>
              </span>
            </div>
          </div>
        )}
      </div>

      {session && (
        <div className="grp">
          <span className="lbl3">Synchronisation</span>
          <div className="card">
            <div className="line">
              <span className="col">
                <span className="t">Synchronisation automatique</span>
                <span className="s">
                  {sync.phase === "error"
                    ? `Erreur : ${sync.detail} — nouvel essai automatique sous 1 min.`
                    : "Ton cerveau suit ton compte sur toutes tes machines, en continu."}
                </span>
              </span>
              <span className="v">
                <button className="btn" onClick={() => void syncNow()} disabled={sync.phase === "syncing"}>
                  {sync.phase === "syncing" ? <Loader2 className="inline size-3 animate-spin" /> : null} Synchroniser
                </button>
              </span>
            </div>
            <div className="line">
              <span className="k">Dernière synchro</span>
              <span className="v">
                {sync.at
                  ? relativeDate(new Date(sync.at).toISOString())
                  : <span className="v mut">jamais</span>}
              </span>
            </div>
            <div className="line">
              <span className="col">
                <span className="t">Sauvegardes manuelles</span>
                <span className="s">
                  {backups.length
                    ? `${backups.length} conservée${backups.length > 1 ? "s" : ""} sur ton compte`
                    : "Aucune pour l'instant — la sync automatique en tient déjà une à jour."}
                </span>
              </span>
              <span className="v">
                <button className="btn" onClick={handleBackup} disabled={busy !== null}>
                  {busy === "backup" ? <Loader2 className="inline size-3 animate-spin" /> : null} Sauvegarder
                </button>
              </span>
            </div>
            {backups.map((b) => (
              <div className="line" key={b.name}>
                <span className="col">
                  <span className="t">{b.created_at ? relativeDate(b.created_at) : b.name}</span>
                  <span className="s">{(b.size / 1024 / 1024).toFixed(1)} Mo</span>
                </span>
                <span className="v">
                  <button className="btn" onClick={() => handleRestore(b.name)} disabled={busy !== null}>
                    {busy === b.name ? <Loader2 className="inline size-3 animate-spin" /> : null} Restaurer
                  </button>
                </span>
              </div>
            ))}
          </div>
          {msg && <p className="note">{msg}</p>}
        </div>
      )}

      <div className="grp">
        <span className="lbl3">Vie privée</span>
        <p className="note">
          L'analyse de tes documents reste sur ta machine. Seul le cerveau — le résultat,
          environ 2 Mo — est synchronisé entre tes appareils.
        </p>
        <div className="card">
          <div className="line">
            <span className="col">
              <span className="t">Envoyer les rapports de crash</span>
              <span className="s">
                Anonymes (aucun contenu de tes notes, chemins masqués). Désactivé par défaut.
                Prend effet au redémarrage — {sentryOn
                  ? "actif sur cette session."
                  : telemetry ? "inactif : redémarre l'app (ou DSN absent du build)." : "inactif."}
              </span>
            </span>
            <button
              className="tog"
              aria-pressed={telemetry}
              aria-label="Envoyer les rapports de crash"
              onClick={async () => {
                const on = !telemetry;
                setTelemetryState(on);
                try { await setTelemetry(on); } catch { setTelemetryState(!on); }
              }}
            >
              <i />
            </button>
          </div>
          {/* Notifications système : la 1re fois, macOS demande l'autorisation.
              En `tauri dev` la notif est attribuée au terminal parent — normal. */}
          <div className="line">
            <span className="col">
              <span className="t">Notifications système</span>
              <span className="s">Lucid t'avertit quand l'Archiviste met ton cerveau à jour.</span>
            </span>
            <span className="v">
              <button className="btn" onClick={() => void notify("Lucid", "Les notifications fonctionnent 🎉")}>
                Tester
              </button>
            </span>
          </div>
        </div>
      </div>

      {/* Diagnostic Archiviste — visible aussi en beta : un rapport RGPD-safe
          (aucun contenu de document, noms anonymisés) à coller dans un retour. */}
      <div className="grp">
        <span className="lbl3">Diagnostic (retour bêta)</span>
        <p className="note">
          Résumé du rangement (compteurs, thèmes, domaines) — <strong>sans aucun contenu de
          tes documents</strong>, noms de dossiers anonymisés. La variante « avec les noms »
          contient tes vrais libellés : elle est faite pour toi, pas pour un retour.
        </p>
        <div className="acts">
          <button
            className="btn"
            disabled={busy !== null}
            onClick={async () => {
              setBusy("diag"); setMsg(null);
              try {
                const report = await archivistDiagnostic(true); // masqué = RGPD-safe
                setMsg(await copyText(report) ? "Diagnostic copié ✓ — colle-le dans ton message." : "Copie impossible.");
              } catch (e) { setMsg(String((e as Error).message ?? e)); }
              finally { setBusy(null); }
            }}
          >
            {busy === "diag" ? <Loader2 className="inline size-3 animate-spin" /> : null} Copier (anonymisé)
          </button>
          <button
            className="btn"
            disabled={busy !== null}
            onClick={async () => {
              setBusy("diag-local"); setMsg(null);
              try {
                const report = await archivistDiagnostic(false); // non masqué = usage local
                setMsg(await copyText(report) ? "Diagnostic local copié ✓ — contient tes vrais noms, garde-le pour toi." : "Copie impossible.");
              } catch (e) { setMsg(String((e as Error).message ?? e)); }
              finally { setBusy(null); }
            }}
          >
            {busy === "diag-local" ? <Loader2 className="inline size-3 animate-spin" /> : null} Copier avec les noms (local)
          </button>
        </div>
      </div>

      {import.meta.env.DEV && (
        <div className="grp">
          <span className="lbl3">Dev uniquement</span>
          <div className="acts">
            <button
              className="btn danger"
              disabled={busy !== null}
              onClick={async () => {
                if (!confirm("Reset complet : cerveau, connecteurs, session, onboarding ET sauvegarde cloud effacés (modèles IA gardés). Continuer ?")) return;
                setBusy("reset"); setMsg(null);
                try {
                  await resetEnvironment();
                  // Vide aussi la sauvegarde cloud (sync.zip) : sinon, une fois reconnecté
                  // au même compte, le premier syncNow() voit un dossier local vide et
                  // retélécharge cette sauvegarde — annulant le reset (le cloud fait foi
                  // par design quand le local est vide, cf. sync.ts). Doit tourner AVANT
                  // le signOut (besoin de la session pour le uid + les droits RLS).
                  const { data: userData } = (await supabase?.auth.getUser()) ?? {};
                  const rid = userData?.user?.id;
                  if (rid) await supabase?.storage.from(BACKUP_BUCKET).remove([`${rid}/${SYNC_FILE}`]);
                  // Déconnexion explicite (pas juste localStorage.clear()) : sans ça la
                  // sync auto retélécharge la dernière sauvegarde du même compte avant
                  // même que l'onboarding s'affiche (constaté par Liam le 2026-07-22).
                  await supabase?.auth.signOut();
                  localStorage.clear(); // flags lucid.* + session Supabase → onboarding complet
                  location.reload();
                } catch (e) { setMsg(String((e as Error).message ?? e)); setBusy(null); }
              }}
            >
              {busy === "reset" ? <Loader2 className="inline size-3 animate-spin" /> : <Trash2 className="inline size-3" />} Tout réinitialiser
            </button>
            <button className="btn" onClick={() => { crashTest().catch((e) => setMsg(String(e))); }}>
              Tester Sentry (panic Rust volontaire)
            </button>
          </div>
        </div>
      )}

      {session && (
        <div className="acts">
          <button className="btn danger" onClick={() => supabase!.auth.signOut()}>Se déconnecter</button>
        </div>
      )}
    </>
  );
}

// ── Section « Mes IA » : une URL MCP unique, pour n'importe quelle IA ────────
// Locale (Claude Desktop/Code sur cette machine) ou distante (claude.ai…) :
// même serveur, même URL, mêmes fonctionnalités (décision 2026-07-21) — plus
// de connexion par client, une seule chose à copier-coller.

export function AiClientsSection() {
  const [mcpUrl, setMcpUrl] = useState<string | null>(null);
  const [stats, setStats] = useState<BrainStats | null>(null);
  const [loadingUrl, setLoadingUrl] = useState(false);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [rotating, setRotating] = useState(false);

  useEffect(() => {
    fetchBrainStats().then(setStats).catch(() => {});
    // Une URL déjà créée doit être VISIBLE sans rien cliquer : sinon révoquer
    // exigeait de d'abord copier son URL, ce qui est absurde quand on veut
    // justement la tuer. Lecture seule — aucune publication du cerveau.
    existingPersonalMcpUrl().then((u) => { if (u) setMcpUrl(u); }).catch(() => {});
  }, []);

  async function copyUrl(u: string) {
    if (await copyText(u)) { setCopied(true); setTimeout(() => setCopied(false), 2000); }
  }

  /** Nouvelle URL, l'ancienne meurt. Le geste utile quand l'URL a fuité :
   *  révoquer seul laisserait les IA muettes sans dire quoi faire ensuite. */
  async function handleRotate() {
    if (!confirm("Regénérer l'URL MCP ? L'ancienne cessera de fonctionner immédiatement — il faudra recoller la nouvelle dans chaque IA connectée.")) return;
    setRotating(true);
    setError(null);
    try {
      const url = await rotatePersonalMcpUrl();
      if (!url) { setError("Regénération impossible — réessaie après une analyse."); return; }
      setMcpUrl(url);
      await copyUrl(url);
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally {
      setRotating(false);
    }
  }

  /** Coupe l'accès sans en rouvrir. Pour « je ne veux plus qu'aucune IA lise
   *  mon cerveau », pas pour une fuite (là, c'est `handleRotate`). */
  async function handleRevoke() {
    if (!confirm("Révoquer l'URL MCP ? Toutes tes IA connectées perdent l'accès à ton cerveau, et aucune nouvelle URL n'est créée.")) return;
    setRotating(true);
    setError(null);
    try {
      await revokePersonalMcpToken();
      setMcpUrl(null);
      localStorage.removeItem("lucid.mcp.connected");
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally {
      setRotating(false);
    }
  }

  async function handleGetUrl() {
    setLoadingUrl(true);
    setError(null);
    try {
      const url = await ensurePersonalMcpUrl();
      if (!url) {
        setError("Indisponible — ton cerveau est vide (lance une analyse d'abord). Si le problème persiste, applique docs/supabase-mcp-personal.sql dans Supabase.");
        return;
      }
      setMcpUrl(url);
      localStorage.setItem("lucid.mcp.connected", "1"); // checklist onboarding (App.tsx)
      await copyUrl(url);
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally {
      setLoadingUrl(false);
    }
  }

  /** L'URL EST un secret : affichée masquée, copiée en clair. Personne n'a
   *  besoin de lire son token à l'écran, et une capture d'écran de Réglages ne
   *  doit pas donner l'accès à un cerveau. */
  const maskedUrl = mcpUrl?.replace(/(token=)[^&]+/, (_m, p) => `${p}${"•".repeat(12)}`) ?? null;

  const totalExchanges = (stats?.days ?? []).reduce((a, b) => a + b, 0);

  return (
    <>
      <div className="grp">
        <div className="sec-top"><span className="h">MCP</span></div>
        <p className="note">
          L'adresse que tu colles dans Claude, ChatGPT ou Cursor pour qu'ils lisent ton
          cerveau. Ils peuvent le consulter et proposer des changements — appliqués
          aussitôt, ou soumis à ta validation (réglage dans « IA locale »).
        </p>
      </div>

      <div className="grp">
        <span className="lbl3">Adresse de ton cerveau</span>
        {mcpUrl ? (
          <>
            <div className="url">
              <span title="Copie pour obtenir l'adresse complète">{maskedUrl}</span>
              {copied && <Check className="size-3.5 text-[var(--color-ok)]" />}
            </div>
            <div className="acts">
              <button className="btn primary" onClick={() => copyUrl(mcpUrl)}>Copier</button>
              <button className="btn" onClick={handleRotate} disabled={rotating}>
                {rotating ? <Loader2 className="inline size-3 animate-spin" /> : null} Régénérer
              </button>
              <button className="btn danger" onClick={handleRevoke} disabled={rotating}>Révoquer</button>
            </div>
            <p className="note warn">
              Régénérer invalide l'ancienne adresse immédiatement : les IA déjà branchées
              devront être reconnectées.
            </p>
          </>
        ) : (
          <div className="acts">
            <button className="btn primary" onClick={handleGetUrl} disabled={loadingUrl}>
              {loadingUrl ? <Loader2 className="inline size-3 animate-spin" /> : null} Créer et copier l'adresse
            </button>
          </div>
        )}
        {error && <p className="note warn">{error}</p>}
      </div>

      <div className="grp">
        <span className="lbl3">Mes IA</span>
        <McpConnectGuide url={mcpUrl} />
      </div>

      <div className="grp">
        <span className="lbl3">Ce que ça a rapporté</span>
        <div className="card">
          <div className="line">
            <span className="k">Échanges</span>
            <span className="v">
              {totalExchanges > 0
                ? `${totalExchanges} sur ${DAYS} jours`
                : <span className="v mut">aucun pour l'instant</span>}
            </span>
          </div>
          <div className="line">
            <span className="k">Tokens évités</span>
            <span className="v">
              {stats?.tokensSaved != null
                ? `${new Intl.NumberFormat("fr-FR", { notation: "compact", maximumFractionDigits: 1 }).format(stats.tokensSaved)}${
                    stats.savedPct != null ? ` · ${Math.round(stats.savedPct)} % de moins` : ""
                  }`
                : <span className="v mut">—</span>}
            </span>
          </div>
        </div>
        <p className="note">
          Compteurs agrégés par jour. Jamais le texte d'une requête, jamais le libellé des
          pages consultées.
        </p>
      </div>
    </>
  );
}

interface Props {
  connectors: ConnectorStatus[];
  spaces: Space[];
  onRefresh: () => void;
  onSyncDone: (hadNew: boolean) => void;
  onClose: () => void;
  onSpaceCreate: (name: string) => void;
  onSpaceRename: (id: string, name: string) => void;
  onSpaceDelete: (id: string) => void;
  /** Appelé après restauration d'une sauvegarde (recharge le graphe). */
  onRestored?: () => void;
}

export function SettingsModal({
  connectors: _connectors, spaces, onRefresh: _onRefresh, onSyncDone: _onSyncDone, onClose,
  onSpaceCreate, onSpaceRename, onSpaceDelete, onRestored,
}: Props) {
  const [section, setSection] = useState<Section>("account");
  const [version, setVersion] = useState("");
  const [email, setEmail] = useState("");
  useEffect(() => { getVersion().then(setVersion).catch(() => {}); }, []);
  useEffect(() => {
    supabase?.auth.getSession().then(({ data }) => setEmail(data.session?.user.email ?? ""));
  }, []);

  // Échap ferme la feuille (comportement attendu de toute surface modale).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Les sources n'apparaissent plus ici : elles sont dans le Launchpad (dock).
  const NAV: { id: Section; label: string; glyph: string }[] = [
    { id: "account", label: "Compte", glyph: "◍" },
    { id: "model",   label: "IA locale", glyph: "◈" },
    { id: "mcp",     label: "MCP", glyph: "◇" },
    { id: "spaces",  label: "Espaces", glyph: "◎" },
  ];

  return (
    <div className="scrim" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="sheet lg" role="dialog" aria-modal="true" aria-label="Réglages">
        <div className="sh-head">
          <span className="t">Réglages</span>
          {email && <span className="pill">{email}</span>}
          <button
            className="d-close"
            style={{ marginLeft: "auto" }}
            onClick={onClose}
            title="Fermer (Échap)"
          >
            Fermer
          </button>
        </div>

        <div className="sh-body">
          <div className="sh-side">
            <span className="pane-lbl">Sections</span>
            <div className="side-list">
              {NAV.map((n) => (
                <button
                  key={n.id}
                  className="side-row"
                  aria-selected={section === n.id || undefined}
                  onClick={() => setSection(n.id)}
                >
                  <span style={{ width: 16, textAlign: "center", color: "var(--sb-label)" }}>{n.glyph}</span>
                  <span className="nm">{n.label}</span>
                </button>
              ))}
            </div>
            <span className="pane-lbl" style={{ marginTop: "auto" }}>
              Lucid{version ? ` · v${version}` : ""}
            </span>
          </div>

          <div className="sh-main">
            {section === "account" && <AccountSection onRestored={onRestored} />}
            {section === "model" && <ModelSection />}
            {section === "mcp" && <AiClientsSection />}
            {section === "spaces" && (
              <SpacesSection spaces={spaces} onCreate={onSpaceCreate} onRename={onSpaceRename} onDelete={onSpaceDelete} />
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
