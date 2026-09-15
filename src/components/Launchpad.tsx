import { useState, useEffect, useCallback } from "react";
import { Loader2, X } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  googleDriveConnect,
  googleDriveDisconnect,
  googleDriveSync,
  googleDriveSelection,
  type DriveSelection,
  importClaudeAi,
  importChatGpt,
  localFolderConnect,
  localFolderList,
  localFolderAdd,
  localFolderRemove,
  localFolderDisconnect,
  localFolderSync,
  obsidianSetVault,
  obsidianVaultPath,
  obsidianDisconnect,
  appleNotesConnect,
  appleNotesSync,
  appleNotesDisconnect,
  claudeCodeAvailable,
  claudeCodeDisconnect,
  claudeCodeReconnect,
} from "@/lib/api";
import { DriveFolderPicker } from "@/components/DriveFolderPicker";
import type { ConnectorStatus } from "@/lib/types";
import { relativeDate } from "@/lib/utils";
import { Tile, connState, connMeta, stateColor } from "@/components/ConnectorTile";
import { IconSearch } from "./icons";

/**
 * Launchpad — la feuille « Connecteurs » de la maquette « l'app vivante » :
 * à gauche les sources ajoutées, à droite la grille de tout ce qui existe, et
 * le détail d'une source à la place de la grille quand on en choisit une.
 *
 * C'est le SEUL endroit où l'on branche/synchronise/retire une source : les
 * Réglages n'en parlent plus (sinon deux écrans racontent des choses
 * différentes du même connecteur). Toute la logique vient de l'ancienne
 * section « Connecteurs » des Réglages — déplacée, pas réécrite.
 */

function Btn({ busy, onClick, primary, danger, children }: {
  busy?: boolean; onClick: () => void; primary?: boolean; danger?: boolean; children: React.ReactNode;
}) {
  return (
    <button
      className={"btn" + (primary ? " primary" : "") + (danger ? " danger" : "")}
      disabled={busy}
      onClick={onClick}
    >
      {busy ? <Loader2 className="inline size-3 animate-spin" /> : null} {children}
    </button>
  );
}

/** D'où viennent les données. Les sources lues sur disque disent leur chemin :
 *  sans ça le détail affiche « Connecté » sans dire connecté à quoi. */
const SOURCE_ORIGIN: Record<string, string> = {
  "claude-code": "~/.claude/projects/",
  cowork: "Claude Desktop → sessions Cowork",
  "claude-ai": "Import ZIP",
  chatgpt: "Import ZIP",
};

interface DetailProps {
  c: ConnectorStatus;
  busy: boolean;
  msg: string;
  obsidianVault?: string | null;
  localFolders?: string[];
  driveSelectionLabel?: string;
  onPickDriveFolders?: () => void;
  onAddFolder?: () => void;
  onRemoveFolder?: (path: string) => void;
  onBack: () => void;
  onConnect: () => void;
  onSync: () => void;
  onDisconnect: () => void;
  onImport: () => void;
}

// `onImport` reste dans les props sans être utilisé : le stand-by du 2026-08-06 a retiré le
// bouton d'import ZIP, pas le circuit. Le garder rend le rallumage immédiat si une API s'ouvre.
function ConnectorDetail({
  c, busy, msg, obsidianVault, localFolders, driveSelectionLabel,
  onPickDriveFolders, onAddFolder, onRemoveFolder, onBack, onConnect, onSync, onDisconnect,
  onImport: _onImport,
}: DetailProps) {
  const st = connState(c);
  const bad = st === "dead";
  const cells: [string, string][] = [
    ["Documents", String(c.conversation_count)],
    ["Contenu le plus récent", c.last_sync ? relativeDate(c.last_sync) : "inconnu"],
  ];
  if (SOURCE_ORIGIN[c.id]) cells.push(["Origine", SOURCE_ORIGIN[c.id]]);
  if (c.id === "obsidian" && obsidianVault) cells.push(["Coffre", obsidianVault.split("/").pop() || obsidianVault]);
  if (c.id === "local-folder" && localFolders) cells.push(["Dossiers suivis", String(localFolders.length)]);

  return (
    <div className="detail">
      <div className="d-top">
        <Tile id={c.id} size="md" />
        <span className="col">
          <span className="nm">{c.name}</span>
          <span className={"d-state" + (bad ? " bad" : "")}>
            <span className="s" style={{ background: stateColor(st) }} />
            {connMeta(c)}
          </span>
        </span>
        <button className="d-close" onClick={onBack}>Toutes les sources</button>
      </div>

      {c.reconnect_reason && (
        <div className="warn-box">
          <span>⚠</span>
          <span>
            Ton accès {c.name} a expiré — la synchronisation est en pause.
            Reconnecte-toi pour la reprendre.
          </span>
        </div>
      )}

      <div className="cells">
        {cells.map(([k, v]) => (
          <div className="cell" key={k}>
            <span className="k">{k}</span>
            <span className="v" title={v}>{v}</span>
          </div>
        ))}
      </div>

      {driveSelectionLabel && c.id === "google-drive" && c.connected && (
        <p className="note">{driveSelectionLabel}</p>
      )}

      {c.id === "local-folder" && c.connected && localFolders && localFolders.length > 0 && (
        <>
          <span className="pane-lbl" style={{ padding: 0 }}>
            {localFolders.length} dossier{localFolders.length > 1 ? "s" : ""} suivi{localFolders.length > 1 ? "s" : ""}
          </span>
          <div className="srcs">
            {localFolders.map((f) => (
              <div className="src-row" key={f}>
                <Tile id={c.id} size="sm" />
                <span className="nm" title={f}>{f}</span>
                <button
                  className="go"
                  onClick={() => onRemoveFolder?.(f)}
                  title="Retirer ce dossier"
                  style={{ color: "var(--sb-label)" }}
                >
                  <X className="size-3" />
                </button>
              </div>
            ))}
          </div>
        </>
      )}

      {/* Une source qui lit peut prendre une minute (Notes Apple passe par
          l'automatisation macOS, un Drive énumère des milliers d'objets) : un
          spinner de 12 px dans un bouton ne se voit pas — il faut le dire en
          toutes lettres (retour de Liam le 2026-09-15). */}
      {busy && (
        <p className="note" style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <Loader2 className="size-3 shrink-0 animate-spin" />
          Lecture de la source en cours — ça peut prendre une minute.
        </p>
      )}
      {msg && <p className="note">{msg}</p>}

      {/* Cowork est détecté seul (le dossier existe ou non) : il n'y a rien à
          connecter, mais laisser la zone d'actions VIDE donnait un panneau qui
          semblait cassé. On dit ce qui se passe. */}
      {c.id === "cowork" && (
        <p className="note">
          {c.connected
            ? `Détecté automatiquement — ${c.conversation_count} session${c.conversation_count > 1 ? "s" : ""} lue${c.conversation_count > 1 ? "s" : ""}. Régénère le cerveau pour les y faire entrer.`
            : "Aucune session Cowork trouvée. Lance une session dans Claude Desktop, elle sera détectée toute seule."}
        </p>
      )}

      {/* Import ZIP retiré (stand-by du 2026-08-06) : aucune API ne permet de lire
          l'historique d'un compte ChatGPT/claude.ai. On dit d'où viennent les
          conversations déjà là, sans proposer d'en réimporter. */}
      {(c.id === "claude-ai" || c.id === "chatgpt") && (
        <p className="note">
          Import d'historique arrêté — {c.conversation_count} conversation
          {c.conversation_count > 1 ? "s" : ""} déjà importée{c.conversation_count > 1 ? "s" : ""} restent
          dans ton cerveau.
        </p>
      )}

      {c.id === "local-folder" && !c.connected && (
        <p className="note">
          Indexe automatiquement ton Bureau, tes Documents et tes Téléchargements
          (PDF, Word, PowerPoint, Excel, CSV — 100 % en local). Les fichiers de
          code/projets ne sont jamais ramassés.
        </p>
      )}

      {c.id === "apple-notes" && !c.connected && (
        <p className="note">
          Lecture seule via l'automatisation native de macOS — jamais la base interne
          de Notes. macOS te demandera d'autoriser Lucid à contrôler Notes.
        </p>
      )}

      <div className="acts">
        {c.id === "claude-code" && (c.connected
          ? <Btn busy={busy} onClick={onDisconnect} danger>Déconnecter</Btn>
          : <Btn busy={busy} onClick={onConnect} primary>Connecter Claude Code</Btn>)}

        {c.id === "google-drive" && !c.connected && (
          <Btn busy={busy} onClick={onConnect} primary>
            {c.reconnect_reason ? "Reconnecter Google Drive" : "Connecter Google Drive"}
          </Btn>
        )}
        {c.id === "google-drive" && c.connected && (
          <>
            <Btn busy={busy} onClick={onSync} primary>Synchroniser</Btn>
            <Btn busy={busy} onClick={() => onPickDriveFolders?.()}>Choisir les dossiers</Btn>
            <Btn busy={busy} onClick={onDisconnect} danger>Déconnecter</Btn>
          </>
        )}

        {c.id === "obsidian" && !c.connected && (
          <Btn busy={busy} onClick={onConnect} primary>Choisir le dossier vault</Btn>
        )}
        {c.id === "obsidian" && c.connected && (
          <>
            <Btn busy={busy} onClick={onConnect}>Changer de vault</Btn>
            <Btn busy={busy} onClick={onDisconnect} danger>Déconnecter</Btn>
          </>
        )}

        {c.id === "local-folder" && !c.connected && (
          <Btn busy={busy} onClick={onConnect} primary>Connecter</Btn>
        )}
        {c.id === "local-folder" && c.connected && (
          <>
            <Btn busy={busy} onClick={onSync} primary>Synchroniser</Btn>
            <Btn busy={busy} onClick={() => onAddFolder?.()}>Ajouter un dossier</Btn>
            <Btn busy={busy} onClick={onDisconnect} danger>Déconnecter</Btn>
          </>
        )}

        {c.id === "apple-notes" && !c.connected && (
          <Btn busy={busy} onClick={onConnect} primary>Connecter Notes Apple</Btn>
        )}
        {c.id === "apple-notes" && c.connected && (
          <>
            <Btn busy={busy} onClick={onSync} primary>Synchroniser</Btn>
            <Btn busy={busy} onClick={onDisconnect} danger>Déconnecter</Btn>
          </>
        )}
      </div>
    </div>
  );
}

export function Launchpad({
  connectors, onRefresh, onSyncDone, onClose, initialId,
}: {
  connectors: ConnectorStatus[];
  onRefresh: () => void;
  onSyncDone: (hadNew: boolean) => void;
  onClose: () => void;
  /** Ouvre directement sur une source (clic sur une tuile du dock, ou
   *  « Reconnecter » depuis une notification). */
  initialId?: string | null;
}) {
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const [msgs, setMsgs] = useState<Record<string, string>>({});
  const [selId, setSelId] = useState<string | null>(initialId ?? null);

  function set(id: string, v: boolean) { setBusy((b) => ({ ...b, [id]: v })); }
  function msg(id: string, m: string)  { setMsgs((s) => ({ ...s, [id]: m })); }

  async function handleImportClaudeAi() {
    const selected = await open({ filters: [{ name: "Export claude.ai", extensions: ["zip"] }], multiple: false });
    if (!selected || typeof selected !== "string") return;
    set("claude-ai", true); msg("claude-ai", "");
    try {
      const count = await importClaudeAi(selected);
      msg("claude-ai", `${count} conversations importées`);
      onRefresh();
    } catch (e) { msg("claude-ai", `Erreur : ${e}`); }
    finally { set("claude-ai", false); }
  }

  async function handleImportChatGpt() {
    const selected = await open({ filters: [{ name: "Export ChatGPT", extensions: ["zip"] }], multiple: false });
    if (!selected || typeof selected !== "string") return;
    set("chatgpt", true); msg("chatgpt", "");
    try {
      const count = await importChatGpt(selected);
      msg("chatgpt", `${count} conversations importées`);
      onRefresh();
    } catch (e) { msg("chatgpt", `Erreur : ${e}`); }
    finally { set("chatgpt", false); }
  }

  // Sélection Drive : `folders` vide = tout le Drive (défaut des comptes connectés
  // avant cette feature — sinon la mise à jour viderait leur Drive au sync suivant).
  const [driveSel, setDriveSel] = useState<DriveSelection | null>(null);
  const [drivePicker, setDrivePicker] = useState(false);
  const refreshDriveSel = useCallback(() => {
    googleDriveSelection().then(setDriveSel).catch(() => {});
  }, []);
  useEffect(() => { refreshDriveSel(); }, [refreshDriveSel]);

  const driveSelectionLabel = !driveSel
    ? undefined
    : driveSel.folders.length === 0
      ? "Tout le Drive est indexé."
      : `${driveSel.folders.length} dossier${driveSel.folders.length > 1 ? "s" : ""} indexé${driveSel.folders.length > 1 ? "s" : ""}${driveSel.include_orphans ? " + fichiers sans dossier" : ""}.`;

  async function handleGoogleConnect() {
    // « En attente du navigateur… » ne disait ni quoi faire ni combien de temps :
    // le testeur du 21/08 ne savait pas si l'OAuth avait réussi ou si c'était
    // lent, et abandonnait. Le côté Rust borne désormais l'attente à 5 min.
    set("google-drive", true);
    msg("google-drive", "Autorise Lucid dans l'onglet qui vient de s'ouvrir, puis reviens ici. (5 min max)");
    try {
      await googleDriveConnect();
      msg("google-drive", "Connecté ! Choisis les dossiers à indexer.");
      onRefresh();
      // Le choix se fait maintenant, pas enterré dans les réglages : c'est le
      // moment où l'utilisateur sait ce qu'il veut donner à Lucid.
      setDrivePicker(true);
    } catch (e) { msg("google-drive", `Erreur : ${e}`); }
    finally { set("google-drive", false); }
  }

  async function handleGoogleSync() {
    set("google-drive", true); msg("google-drive", "Synchronisation…");
    try {
      const [newFiles, total, unreadable] = await googleDriveSync();
      // Le muet est dit : sinon un document présent dans Drive mais dont on n'a
      // extrait aucun texte n'arrive jamais dans le cerveau sans explication.
      const mute = unreadable > 0 ? ` — ${unreadable} sans texte exploitable` : "";
      msg("google-drive", (newFiles > 0 ? `${newFiles} nouveaux sur ${total}` : `${total} fichiers en cache`) + mute);
      onRefresh();
      if (total > 0) ingest(newFiles > 0);
    } catch (e) { msg("google-drive", `Erreur : ${e}`); }
    finally { set("google-drive", false); }
  }

  async function handleGoogleDisconnect() {
    set("google-drive", true);
    try {
      await googleDriveDisconnect();
      msg("google-drive", "Déconnecté");
      onRefresh();
    } catch (e) { msg("google-drive", `Erreur : ${e}`); }
    finally { set("google-drive", false); }
  }

  const [obsidianVault, setObsidianVault] = useState<string | null>(null);
  useEffect(() => { obsidianVaultPath().then(setObsidianVault); }, []);

  async function handleObsidianConnect() {
    const selected = await open({ directory: true, multiple: false });
    if (!selected || typeof selected !== "string") return;
    set("obsidian", true); msg("obsidian", "");
    try {
      await obsidianSetVault(selected);
      setObsidianVault(selected);
      msg("obsidian", "Vault configuré — analyse en cours…");
      onRefresh();
      ingest(true);
    } catch (e) { msg("obsidian", `Erreur : ${e}`); }
    finally { set("obsidian", false); }
  }

  async function handleObsidianDisconnect() {
    set("obsidian", true);
    try {
      await obsidianDisconnect();
      setObsidianVault(null);
      msg("obsidian", "Déconnecté");
      onRefresh();
    } catch (e) { msg("obsidian", `Erreur : ${e}`); }
    finally { set("obsidian", false); }
  }

  const [localFolders, setLocalFolders] = useState<string[]>([]);
  useEffect(() => { localFolderList().then(setLocalFolders); }, []);

  async function handleLocalFolderConnect() {
    set("local-folder", true); msg("local-folder", "");
    try {
      const folders = await localFolderConnect();
      setLocalFolders(folders);
      msg("local-folder", `${folders.length} dossier${folders.length > 1 ? "s" : ""} détecté${folders.length > 1 ? "s" : ""} — lecture en cours…`);
      onRefresh();
      await handleLocalFolderSync(); // enchaîne le scan : « connecté » sans contenu ne veut rien dire
    } catch (e) { msg("local-folder", `Erreur : ${e}`); }
    finally { set("local-folder", false); }
  }

  async function handleLocalFolderAddFolder() {
    const selected = await open({ directory: true, multiple: false });
    if (!selected || typeof selected !== "string") return;
    set("local-folder", true); msg("local-folder", "");
    try {
      const folders = await localFolderAdd(selected);
      setLocalFolders(folders);
      msg("local-folder", "Dossier ajouté — lance une synchronisation.");
    } catch (e) { msg("local-folder", `Erreur : ${e}`); }
    finally { set("local-folder", false); }
  }

  async function handleLocalFolderRemoveFolder(path: string) {
    set("local-folder", true);
    try {
      const folders = await localFolderRemove(path);
      setLocalFolders(folders);
      msg("local-folder", "Dossier retiré.");
    } catch (e) { msg("local-folder", `Erreur : ${e}`); }
    finally { set("local-folder", false); }
  }

  async function handleLocalFolderSync() {
    set("local-folder", true); msg("local-folder", "Synchronisation… (l'extraction PDF peut être longue)");
    try {
      const r = await localFolderSync();
      const skipped = r.skipped.length ? ` — ${r.skipped.length} illisibles` : "";
      // Un dossier refusé par l'OS ne se compte pas avec les fichiers illisibles :
      // son contenu n'a même pas été vu, et il y a un geste à faire pour l'ouvrir.
      const denied = r.denied?.length
        ? ` — ${r.denied.length} dossier${r.denied.length > 1 ? "s" : ""} sans autorisation (Réglages Système › Fichiers et dossiers)`
        : "";
      msg("local-folder", (r.new > 0 ? `${r.new} nouveaux sur ${r.total}` : `${r.total} fichiers indexés`) + skipped + denied);
      if (r.skipped.length) console.warn("Dossiers locaux — fichiers ignorés :", r.skipped);
      if (r.denied?.length) console.warn("Dossiers locaux — accès refusé :", r.denied);
      onRefresh();
      if (r.total > 0) ingest(r.new > 0);
    } catch (e) { msg("local-folder", `Erreur : ${e}`); }
    finally { set("local-folder", false); }
  }

  async function handleLocalFolderDisconnect() {
    set("local-folder", true);
    try {
      await localFolderDisconnect();
      setLocalFolders([]);
      msg("local-folder", "Déconnecté");
      onRefresh();
    } catch (e) { msg("local-folder", `Erreur : ${e}`); }
    finally { set("local-folder", false); }
  }

  async function handleAppleNotesConnect() {
    set("apple-notes", true); msg("apple-notes", "");
    try {
      const n = await appleNotesConnect();
      msg("apple-notes", `${n} note${n > 1 ? "s" : ""} importée${n > 1 ? "s" : ""} — analyse en cours…`);
      onRefresh();
      ingest(n > 0);
    } catch (e) { msg("apple-notes", `Erreur : ${e}`); }
    finally { set("apple-notes", false); }
  }

  async function handleAppleNotesSync() {
    set("apple-notes", true); msg("apple-notes", "Synchronisation…");
    try {
      const n = await appleNotesSync();
      msg("apple-notes", `${n} note${n > 1 ? "s" : ""} synchronisée${n > 1 ? "s" : ""}`);
      onRefresh();
      ingest(n > 0);
    } catch (e) { msg("apple-notes", `Erreur : ${e}`); }
    finally { set("apple-notes", false); }
  }

  async function handleAppleNotesDisconnect() {
    set("apple-notes", true);
    try {
      await appleNotesDisconnect();
      msg("apple-notes", "Déconnecté");
      onRefresh();
    } catch (e) { msg("apple-notes", `Erreur : ${e}`); }
    finally { set("apple-notes", false); }
  }

  async function handleClaudeCodeDisconnect() {
    set("claude-code", true);
    try {
      await claudeCodeDisconnect();
      msg("claude-code", "Déconnecté");
      onRefresh();
    } catch (e) { msg("claude-code", `Erreur : ${e}`); }
    finally { set("claude-code", false); }
  }

  async function handleClaudeCodeConnect() {
    set("claude-code", true);
    try {
      await claudeCodeReconnect();
      const ok = await claudeCodeAvailable();
      msg("claude-code", ok ? "Connecté — analyse en cours…" : "Dossier ~/.claude/projects introuvable — Claude Code est-il installé ?");
      onRefresh();
      if (ok) ingest(true);
    } catch (e) { msg("claude-code", `Erreur : ${e}`); }
    finally { set("claude-code", false); }
  }

  /** Une source vient d'être branchée : ce qu'elle apporte doit ENTRER dans le
   *  cerveau dans la foulée. Sans ça « connectée » et « rien dans la carte »
   *  sont indiscernables, et il fallait cliquer « Synchroniser » pour que quoi
   *  que ce soit arrive (retour de Liam le 2026-09-15, sur Notes Apple).
   *  La feuille s'efface : l'analyse qui démarre a son objet « ça avance » —
   *  l'orbe et sa barre — et il est derrière cette feuille. */
  function ingest(hadNew: boolean) {
    onSyncDone(hadNew);
    onClose();
  }

  const [query, setQuery] = useState("");
  const sel = selId ? connectors.find((c) => c.id === selId) ?? null : null;
  const added = connectors.filter((c) => c.connected || c.reconnect_reason);
  const q = query.trim().toLowerCase();
  const grid = connectors.filter((c) => !q || c.name.toLowerCase().includes(q));

  // Échap ferme la feuille (et le sélecteur Drive gère le sien).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") { e.stopPropagation(); onClose(); } };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  }, [onClose]);

  return (
    <div className="scrim" onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="sheet lg" role="dialog" aria-modal="true" aria-label="Connecteurs">
        <div className="sh-head">
          <span className="t">Connecteurs</span>
          <span className="pill">{added.length} connectés</span>
          <label className="sh-search">
            <input
              type="text"
              value={query}
              onChange={(e) => { setQuery(e.target.value); setSelId(null); }}
              placeholder="Chercher une source…"
              aria-label="Chercher une source"
            />
            <IconSearch className="size-[14px]" />
          </label>
        </div>

        <div className="sh-body">
          <div className="sh-side">
            <span className="pane-lbl">Ajoutés</span>
            <div className="side-list">
              {added.map((c) => (
                <button
                  key={c.id}
                  className="side-row"
                  aria-selected={c.id === selId || undefined}
                  onClick={() => { setSelId(c.id); setDrivePicker(false); }}
                >
                  <Tile id={c.id} size="sm" />
                  <span className="nm">{c.name}</span>
                  {c.conversation_count > 0 && <span className="n">{c.conversation_count}</span>}
                  <span className="sd" style={{ background: stateColor(connState(c)) }} />
                </button>
              ))}
              {added.length === 0 && (
                <p className="note" style={{ padding: "0 8px" }}>Aucune source connectée pour l'instant.</p>
              )}
            </div>
            <button className="side-add" onClick={() => setSelId(null)}>+ Ajouter une source</button>
          </div>

          <div className="sh-main">
            {drivePicker && sel?.id === "google-drive" ? (
              <DriveFolderPicker
                onClose={() => setDrivePicker(false)}
                onSaved={(m) => {
                  // Choisir des dossiers sans rien en faire laissait le Drive
                  // « connecté » et le cerveau vide : on enchaîne la synchro.
                  msg("google-drive", m);
                  refreshDriveSel();
                  setDrivePicker(false);
                  void handleGoogleSync();
                }}
              />
            ) : sel ? (
              <ConnectorDetail
                c={sel}
                busy={busy[sel.id] ?? false}
                msg={msgs[sel.id] ?? ""}
                obsidianVault={obsidianVault}
                localFolders={localFolders}
                driveSelectionLabel={driveSelectionLabel}
                onPickDriveFolders={() => { setSelId("google-drive"); setDrivePicker(true); }}
                onAddFolder={handleLocalFolderAddFolder}
                onRemoveFolder={handleLocalFolderRemoveFolder}
                onBack={() => setSelId(null)}
                onConnect={
                  sel.id === "claude-code" ? handleClaudeCodeConnect :
                  sel.id === "obsidian" ? handleObsidianConnect :
                  sel.id === "local-folder" ? handleLocalFolderConnect :
                  sel.id === "apple-notes" ? handleAppleNotesConnect :
                  handleGoogleConnect
                }
                onSync={
                  sel.id === "local-folder" ? handleLocalFolderSync :
                  sel.id === "apple-notes" ? handleAppleNotesSync :
                  handleGoogleSync
                }
                onDisconnect={
                  sel.id === "claude-code" ? handleClaudeCodeDisconnect :
                  sel.id === "obsidian" ? handleObsidianDisconnect :
                  sel.id === "local-folder" ? handleLocalFolderDisconnect :
                  sel.id === "apple-notes" ? handleAppleNotesDisconnect :
                  handleGoogleDisconnect
                }
                onImport={sel.id === "chatgpt" ? handleImportChatGpt : handleImportClaudeAi}
              />
            ) : (
              <>
                <span className="pane-lbl" style={{ padding: 0 }}>Toutes les sources</span>
                {grid.length ? (
                  <div className="lp-grid">
                    {grid.map((c) => (
                      <button key={c.id} className="g-item" onClick={() => setSelId(c.id)}>
                        <Tile id={c.id} size="big" badge={c.connected} />
                        <span className="lbl2">{c.name}</span>
                        <span className="sub2">
                          {c.connected
                            ? (c.conversation_count ? `${c.conversation_count} ajoutés` : "Ajouté")
                            : "Ajouter"}
                        </span>
                      </button>
                    ))}
                  </div>
                ) : (
                  <p className="note">Aucune source ne correspond.</p>
                )}
              </>
            )}
          </div>
        </div>
      </div>

    </div>
  );
}
