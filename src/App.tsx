import { useEffect, useMemo, useRef, useState } from "react";
import {
  Network,
  FolderTree,
  FileText,
  RefreshCw,
  Search,
  Settings,
  Layers,
  MessageCircle,
  History,
  RotateCcw,
  Plus,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { BrainMap, isLeafKind } from "@/components/BrainMap";
import { CommandPalette } from "@/components/CommandPalette";
import { FolderView } from "@/components/FolderView";
import { SpacesPanel, AssistantPanel } from "@/components/LeftSidebar";
import { SettingsModal } from "@/components/SettingsModal";
import { ThemeToggle } from "@/components/ThemeToggle";
import { UpdateBanner } from "@/components/UpdateBanner";
import { BetaBadge } from "@/components/BetaBadge";
import {
  GenerateEmpty,
  GenerateProgress,
  ScanSteps,
  MarkdownView,
} from "@/components/BrainView";
import { NodeDetail } from "@/components/NodeDetail";
import { NodePicker } from "@/components/NodePicker";
import { StarterChecklist, type ChecklistItem } from "@/components/StarterChecklist";
import { TimelineBar } from "@/components/TimelineBar";
import {
  generateBrain,
  readBrainGraph,
  connectorsStatus,
  googleDriveSync,
  aiSetupNeeded,
  listSnapshots,
  restoreSnapshot,
  listSpaces,
  createSpace,
  addNodeToSpace,
  deleteSpace,
  renameSpace,
  createNoteNode,
  deleteNode,
  importFile,
  listMcpProposals,
  resolveMcpProposal,
  resolveAllMcpProposals,
  mcpManualValidationEnabled,
  importMcpProposal,
  setNodeParent,
  seedDemo,
  localFolderConnect,
  localFolderSync,
  obsidianAutoConnect,
  appleNotesConnect,
  claudeCodeAvailable,
  claudeCodeReconnect,
  type BrainProgress,
  type LocalFolderProgress,
} from "@/lib/api";
import { syncNow } from "@/lib/sync"; // startAutoSync désactivé temporairement, cf. plus bas
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { ShareModal } from "@/components/ShareModal";
import { RemoteSpaceView } from "@/components/RemoteSpaceView";
import { fetchSharedWithMe, ensurePersonalMcpSpace, type SharedWithMe } from "@/lib/share";
import { supabase } from "@/lib/supabase";
import type { McpProposal, SnapshotInfo, Space } from "@/lib/types";
import { SetupScreen } from "@/components/SetupScreen";
import type {
  BrainGraph,
  BrainNode,
  ConnectorStatus,
} from "@/lib/types";
import { cn, relativeDate } from "@/lib/utils";

type View = "map" | "folder" | "brain";

function filterGraphBySpace(graph: BrainGraph, nodeIds: string[]): BrainGraph {
  const idSet = new Set(nodeIds);
  const byId = new Map(graph.nodes.map((n) => [n.id, n]));
  function addAncestors(id: string) {
    const n = byId.get(id);
    if (!n?.parent_id) return;
    idSet.add(n.parent_id);
    addAncestors(n.parent_id);
  }
  nodeIds.forEach((id) => addAncestors(id));
  const filtered = new Set([...idSet]);
  const nodes = graph.nodes.filter((n) => filtered.has(n.id));
  const nodeSet = new Set(nodes.map((n) => n.id));
  const edges = graph.edges.filter((e) => nodeSet.has(e.source) && nodeSet.has(e.target));
  return { ...graph, nodes, edges };
}

/** Notification macOS/Windows native — l'Archiviste en mode autonome n'affiche
 *  rien à l'écran (ni bulle ni panneau), donc sans ça l'utilisateur n'a aucun
 *  moyen de savoir qu'une action a eu lieu, ou qu'une proposition reste bloquée
 *  (demandé par Liam le 2026-07-21). Best-effort : jamais bloquant si refusée. */
async function notify(title: string, body: string) {
  try {
    let granted = await isPermissionGranted();
    if (!granted) granted = (await requestPermission()) === "granted";
    if (granted) sendNotification({ title, body });
  } catch { /* pas grave, juste pas de notification cette fois */ }
}

/** Libellés FR du panneau Historique — le plus récent tagué "mcp_accept" est
 *  de facto « annuler la dernière action de l'Archiviste » (bouton Restaurer). */
const SNAPSHOT_REASON_LABELS: Record<string, string> = {
  mcp_accept: "Archiviste (MCP)",
  delete_node: "Suppression",
  pre_restore: "Avant restauration",
  regenerate: "Régénération",
  manual: "Manuel",
};

/** Libellé + détail d'une proposition MCP dans le panneau, selon son `action`
 *  (create garde le comportement historique : titre = label, détail = parent). */
function describeProposal(
  p: import("@/lib/types").McpProposal,
  labelOf: (id: string) => string,
): { title: string; detail: string } {
  switch (p.action) {
    case "update":
      return { title: `Modifier « ${labelOf(p.target_id)} »`, detail: "" };
    case "move":
      return { title: `Déplacer « ${labelOf(p.target_id)} »`, detail: `→ ${labelOf(p.new_parent_id)}` };
    case "merge": {
      const [survivor, ...rest] = p.merge_ids;
      return {
        title: `Fusionner ${p.merge_ids.length} pages`,
        detail: `${rest.map(labelOf).join(", ")} → ${labelOf(survivor)}`,
      };
    }
    case "link":
      return { title: `Lier « ${labelOf(p.target_id)} » ↔ « ${labelOf(p.link_target)} »`, detail: p.relation || "" };
    default:
      return { title: p.label, detail: `→ ${labelOf(p.parent_id)}` };
  }
}

function App() {
  const [view, setView]       = useState<View>("map");
  const [query, setQuery]     = useState("");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [focus, setFocus]     = useState<{ id: string; k: number } | null>(null);
  const [leftPanel, setLeftPanel]   = useState<"spaces" | "assistant" | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);

  const [needsSetup, setNeedsSetup] = useState<boolean | null>(null);
  const [graph, setGraph]           = useState<BrainGraph | null>(null);
  const [revealKey, setRevealKey]   = useState(0);
  const [connectors, setConnectors] = useState<ConnectorStatus[]>([]);
  const [generating, setGenerating] = useState(false);
  // Scan initial (bouton "Commencer à créer mon cerveau") : connecte + synchronise
  // les dossiers locaux avec un retour en direct, avant d'enchaîner sur la génération.
  const [scanning, setScanning] = useState(false);
  const [scanProgress, setScanProgress] = useState<LocalFolderProgress | null>(null);
  // Fichiers vus pendant le scan → bulles-feuilles synthétiques sous un root
  // "Lucid" (mêmes composants/animation de pop que le vrai graphe — pas un
  // effet séparé) : voir `scanGraph` plus bas.
  const [scanFiles, setScanFiles] = useState<string[]>([]);
  // Auto-détections annexes affichées en direct (Obsidian, Notes Apple, Claude Desktop).
  const [scanSteps, setScanSteps] = useState<string[]>([]);
  // Genesis = la carte se construit à l'écran (1er cerveau). En régénération
  // (re-sync), on garde la carte affichée et on ajoute les nouveaux nœuds.
  const [genesisRun, setGenesisRun] = useState(false);
  const [progress, setProgress]   = useState<BrainProgress | null>(null);
  // Graphe vivant : état provisoire réel émis par le pipeline pendant la génération.
  const [partialGraph, setPartialGraph] = useState<BrainGraph | null>(null);
  // Timeline : curseur temporel (epoch ms) ; null = timeline fermée.
  const [timeCutoff, setTimeCutoff] = useState<number | null>(null);
  const [error, setError]         = useState<string | null>(null);

  const [selectedNode, setSelectedNode] = useState<BrainNode | null>(null);
  const [nodeExpanded, setNodeExpanded] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [snapshots, setSnapshots] = useState<SnapshotInfo[]>([]);
  const [restoring, setRestoring] = useState(false);

  const [spaces, setSpaces] = useState<Space[]>([]);
  const [activeSpaceId, setActiveSpaceId] = useState<string | null>(null);

  const [noteOpen, setNoteOpen] = useState(false);
  const [noteTitle, setNoteTitle] = useState("");
  const [noteParent, setNoteParent] = useState("");
  const [importing, setImporting] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);
  const [parentPickerOpen, setParentPickerOpen] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const toastTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Mode autonome : `proposals` reste toujours vide (cf. le poll plus bas) →
  // aucune bulle/badge/panneau ne s'affiche jamais, l'IA écrit silencieusement.
  // Mode validation manuelle : affichage inchangé (bulle ambre, badge orange, panneau).
  const [proposals, setProposals] = useState<McpProposal[]>([]);

  // ── Checklist « Bien démarrer » : flags persistés + statut IA ──
  const [uxFlags, setUxFlags] = useState(() => ({
    openedPage: localStorage.getItem("lucid.flag.openedPage") === "1",
    importedFile: localStorage.getItem("lucid.flag.importedFile") === "1",
  }));
  function setUxFlag(k: keyof typeof uxFlags) {
    localStorage.setItem(`lucid.flag.${k}`, "1");
    setUxFlags((f) => (f[k] ? f : { ...f, [k]: true }));
  }
  const [checklistDismissed, setChecklistDismissed] = useState(
    () => localStorage.getItem("lucid.checklist.dismissed") === "1",
  );

  const [booted, setBooted] = useState(false); // évite le flash avant le chargement initial
  // Mode démo : carte explorable sans connecteur, données jetables (reset à la sortie).
  const [demoMode, setDemoMode] = useState(() => localStorage.getItem("lucid.demo") === "1");
  async function handleSeedDemo() {
    const g = await seedDemo();
    setGraph(g);
    setRevealKey((k) => k + 1);
    await listSpaces().then(setSpaces);
    localStorage.setItem("lucid.demo", "1");
    setDemoMode(true);
  }

  /** Bouton "Commencer à créer mon cerveau" : connecte + synchronise les dossiers
   *  locaux (Bureau/Documents/Téléchargements) avec un retour fichier par fichier
   *  en direct, puis enchaîne sur la génération du cerveau. Le 1er accès à ces
   *  dossiers déclenche la demande d'autorisation native de l'OS (Mac : prompts
   *  TCC par dossier ; Windows : accès direct, pas de prompt système). */
  async function handleStartScan() {
    setScanning(true);
    setError(null);
    setScanSteps([]);
    setGenesisRun(!graph); // même mise en scène "genesis" que la 1ère génération, dès le scan
    setScanFiles([]);
    const unlisten = await listen<LocalFolderProgress>("local-folder-progress", (e) => {
      setScanProgress(e.payload);
      setScanFiles((prev) => [...prev, e.payload.label]);
    });
    let ok = true;
    try {
      // Claude Code : opt-in par défaut sur un compte neuf (vie privée — voir
      // set_active_user_in côté Rust, qui pose un flag "disabled" au 1er login).
      // Le scan machine est le geste "tout récupérer" : `reconnect` retire ce
      // flag s'il existe (no-op sinon, sûr à appeler même si le dossier est
      // absent) — puis on vérifie que ~/.claude/projects existe vraiment.
      try {
        await claudeCodeReconnect();
        if (await claudeCodeAvailable()) {
          setScanSteps((s) => [...s, "Claude Code connecté"]);
        }
      } catch { /* absent sur cette machine */ }

      // Obsidian : détecte le vault le plus récent si Obsidian a déjà tourné
      // sur cette machine (lit sa propre config, pas de dialogue à l'user).
      try {
        const vault = await obsidianAutoConnect();
        if (vault) setScanSteps((s) => [...s, `Obsidian connecté (${vault.split(/[/\\]/).pop()})`]);
      } catch { /* absent ou déjà configuré autrement : silencieux, pas une erreur */ }

      await localFolderConnect();
      await localFolderSync();
      setScanSteps((s) => [...s, "Bureau, Documents et Téléchargements scannés"]);

      // Notes Apple : Mac uniquement. Sur Windows ou si l'autorisation est
      // refusée, l'échec est attendu — on ne bloque pas le scan pour ça
      // (gérable ensuite depuis les Réglages, qui affiche le vrai message).
      try {
        const n = await appleNotesConnect();
        setScanSteps((s) => [...s, `${n} note${n > 1 ? "s" : ""} Apple importée${n > 1 ? "s" : ""}`]);
      } catch { /* non-Mac ou autorisation refusée */ }

      connectorsStatus().then(setConnectors);
    } catch (e) {
      ok = false;
      setError(String(e));
    } finally {
      unlisten();
      setScanning(false);
      setScanProgress(null);
    }
    if (ok) await handleGenerate();
  }
  // Premier lancement (modèle prêt, pas de cerveau, jamais onboardé) : plus de
  // seed automatique (décision Liam, 2026-07-21) — l'utilisateur atterrit sur
  // l'écran GenerateEmpty et choisit explicitement : scanner sa machine ou
  // explorer la démo. La modale "brancher ses IA" a été retirée (2026-07-21) :
  // un seul MCP distant maintenant (2026-07-21) — plus de connexion silencieuse
  // par client, juste une URL à copier dans Réglages ; on retient ce cap franchi
  // en local (pas de round-trip réseau pour une simple coche de checklist).
  const [aiConnected, setAiConnected] = useState(false);
  useEffect(() => {
    if (needsSetup) return;
    setAiConnected(localStorage.getItem("lucid.mcp.connected") === "1");
  }, [needsSetup, settingsOpen]); // re-check quand on ferme les Settings

  const rootId = useMemo(() => graph?.nodes.find((n) => n.kind === "root")?.id ?? "", [graph]);
  const noteParents = useMemo(
    () => graph?.nodes.filter((n) => !isLeafKind(n.kind)) ?? [],
    [graph],
  );
  const lastSync = useMemo(() => {
    const syncs = connectors.filter((c) => c.last_sync).map((c) => c.last_sync!).sort();
    return syncs.length ? syncs[syncs.length - 1] : null;
  }, [connectors]);

  // Graphe synthétique du scan : un root "Lucid" + une bulle-feuille par fichier
  // déjà vu. Mêmes composants/animations que le vrai graphe (BrainMap ne fait
  // aucune différence) — donc "petit à petit, comme le canvas fini", pas un
  // effet séparé. Remplacé par le vrai graphe dès que la génération démarre.
  const scanGraph: BrainGraph = useMemo(() => ({
    nodes: [
      { id: "scan-root", label: "Lucid", kind: "root", weight: 0, summary: "", keywords: [], decisions: [], patterns: [], community: 0, parent_id: null, synthesized_at: null, content: "" },
      ...scanFiles.map((f, i) => ({
        id: `scan-${i}`, label: f.split(/[/\\]/).pop() || f, kind: "leaf" as const, weight: 0,
        summary: "", keywords: [], decisions: [], patterns: [], community: 0, parent_id: "scan-root", synthesized_at: null, content: "",
      })),
    ],
    edges: [], markdown: "", report: "", generated_at: "",
  }), [scanFiles]);

  // Petite phrase « Lucid » affichée sous le root pendant le scan/une régénération.
  const busyMessage = useMemo(() => {
    if (scanning) {
      return scanProgress ? `Lucid scanne tes fichiers (${scanProgress.current}/${scanProgress.total})` : "Lucid se prépare";
    }
    if (!generating) return null;
    if (!progress) return "Lucid se prépare";
    if (progress.label.startsWith("Synthèse")) return "Lucid tisse l'arborescence";
    return `Lucid analyse tes contenus (${progress.current}/${progress.total})`;
  }, [scanning, scanProgress, generating, progress]);

  useEffect(() => {
    // L'IA locale est optionnelle : si l'user a passé le setup, on n'affiche
    // plus l'écran de téléchargement (installable plus tard via les Réglages).
    aiSetupNeeded().then((n) =>
      setNeedsSetup(n && localStorage.getItem("lucid.ai.skipped") !== "1"),
    );
    readBrainGraph().then((g) => {
      if (g) { setGraph(g); setRevealKey((k) => k + 1); }
      setBooted(true); // graphe initial chargé (ou absent) → évite le flash de GenerateEmpty
    });
    connectorsStatus().then(setConnectors);
    listSpaces().then(setSpaces);
  }, []);

  useEffect(() => {
    const unlisten = listen<BrainProgress>("brain-progress", (e) =>
      setProgress(e.payload),
    );
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  // Graphe vivant : le pipeline émet des graphes provisoires (nœuds allégés,
  // toutes les props BrainNode requises sont présentes côté Rust).
  useEffect(() => {
    const unlisten = listen<{ nodes: BrainNode[]; edges: BrainGraph["edges"] }>(
      "brain-partial",
      (e) => setPartialGraph({ ...e.payload, markdown: "", report: "", generated_at: "" }),
    );
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  // Watch auto : le backend a régénéré tout seul (session Claude Code modifiée)
  // → recharger le graphe ; les nouvelles bulles pop via l'animation en cascade.
  useEffect(() => {
    const unlisten = listen("brain-updated", () => {
      readBrainGraph().then((g) => { if (g) setGraph(g); });
      connectorsStatus().then(setConnectors);
      void syncNow();
    });
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  // Sync cloud : le cerveau suit le compte entre machines (événementiel :
  // watcher local + ping Realtime, filets boot/60 s/focus/fermeture — voir lib/sync.ts).
  // ponytail: désactivée temporairement (bug pendant la démo) — réactiver en
  // remettant l'appel à startAutoSync ci-dessous.
  useEffect(() => {
    return undefined;
    /*
    return startAutoSync(() => {
      // Un pull a fusionné du contenu : c'est un vrai cerveau, plus la démo.
      setDemoMode(false);
      readBrainGraph().then((g) => {
        if (!g) return;
        setGraph((prev) => {
          // Re-animation (revealKey) seulement si la structure change : un pull
          // de contenu ne doit pas vider/reconstruire le canvas sous l'utilisateur.
          const ids = (x: BrainGraph) => x.nodes.map((n) => n.id).sort().join("\n");
          if (!prev || ids(prev) !== ids(g)) setRevealKey((k) => k + 1);
          return g;
        });
      });
      listSpaces().then(setSpaces);
      connectorsStatus().then(setConnectors);
    });
    */
  }, []);

  // `skipSync` : la sync a déjà été faite par le connecteur (bouton Synchroniser)
  // → on régénère seulement, sans re-synchroniser tous les connecteurs.
  async function handleGenerate(opts?: { skipSync?: boolean }) {
    setGenesisRun(!graph); // genesis uniquement s'il n'y a pas encore de carte
    setGenerating(true);
    setError(null);
    setProgress(null);
    setPartialGraph(null);
    setTimeCutoff(null);
    try {
      // Sync tous les connecteurs connectés avant de régénérer.
      if (!opts?.skipSync) {
        const syncs: Promise<unknown>[] = [];
        for (const c of connectors) {
          if (!c.connected) continue;
          if (c.id === "google-drive") syncs.push(googleDriveSync().catch(() => {}));
        }
        if (syncs.length) await Promise.all(syncs);
      }

      setGraph(await generateBrain());
      setRevealKey((k) => k + 1);
      connectorsStatus().then(setConnectors);
      // Le cerveau réel remplace le contenu starter (côté Rust, demo.flag est retiré).
      localStorage.removeItem("lucid.demo");
      setDemoMode(false);
      void syncNow(); // nouveau cerveau → dispo sur les autres machines sans attendre le tick
    } catch (e) {
      setError(String(e));
    } finally {
      setGenerating(false);
      setProgress(null);
      setPartialGraph(null);
    }
  }

  function selectNode(n: BrainNode) {
    setSelectedNode(n);
    setUxFlag("openedPage");
  }
  function closeDetail() {
    setSelectedNode(null);
    setNodeExpanded(false);
  }

  function closePalette() {
    setPaletteOpen(false);
    setQuery("");
  }

  function handlePaletteSelect(n: BrainNode) {
    selectNode(n);
    setFocus({ id: n.id, k: Date.now() });
    closePalette();
  }

  // Listener clavier global unique : ⌘K (palette) + Esc (fermetures en cascade).
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        if (paletteOpen) closePalette();
        else { setQuery(""); setPaletteOpen(true); }
        return;
      }
      if (e.key !== "Escape") return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
      if (settingsOpen) setSettingsOpen(false);
      else if (paletteOpen) closePalette();
      else if (nodeExpanded) setNodeExpanded(false);
      else if (selectedNode) closeDetail();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [settingsOpen, paletteOpen, nodeExpanded, selectedNode]);

  async function handleOpenHistory() {
    setHistoryOpen((o) => !o);
    const list = await listSnapshots();
    setSnapshots(list);
  }

  async function handleRestore(snapshotId: string) {
    setRestoring(true);
    try {
      const g = await restoreSnapshot(snapshotId);
      setGraph(g);
      setRevealKey((k) => k + 1);
      setHistoryOpen(false);
    } finally {
      setRestoring(false);
    }
  }

  function handleContentSaved(nodeId: string, content: string) {
    setGraph((prev) => {
      if (!prev) return prev;
      return { ...prev, nodes: prev.nodes.map((n) => n.id === nodeId ? { ...n, content } : n) };
    });
  }

  function handleNodeRenamed(nodeId: string, label: string) {
    setGraph((prev) => prev ? { ...prev, nodes: prev.nodes.map((n) => n.id === nodeId ? { ...n, label } : n) } : prev);
    setSelectedNode((prev) => prev && prev.id === nodeId ? { ...prev, label } : prev);
  }

  async function handleSpaceCreate(name: string) {
    const s = await createSpace(name);
    setSpaces((prev) => [...prev, s]);
  }

  // Modale de partage (public / privé sur invitation).
  const [shareSpace, setShareSpace] = useState<Space | null>(null);
  // Spaces partagés avec moi (invité) + celui ouvert en lecture seule.
  const [sharedWithMe, setSharedWithMe] = useState<SharedWithMe[]>([]);
  const [remoteSpaceId, setRemoteSpaceId] = useState<string | null>(null);
  useEffect(() => {
    if (leftPanel !== "spaces") return;
    fetchSharedWithMe().then(setSharedWithMe).catch(() => {});
  }, [leftPanel]);

  async function handleSpaceRename(id: string, name: string) {
    await renameSpace(id, name);
    setSpaces((prev) => prev.map((s) => (s.id === id ? { ...s, name } : s)));
  }

  async function handleAddNodeToSpace(nodeId: string, spaceId: string) {
    // Le backend ajoute le sous-arbre entier → on relit plutôt que de deviner.
    await addNodeToSpace(spaceId, nodeId);
    setSpaces(await listSpaces());
  }

  async function refreshGraph() {
    const g = await readBrainGraph();
    if (!g) return;
    setGraph((prev) => {
      // Ne relance l'animation d'apparition (revealKey) QUE si l'ensemble des
      // nœuds a vraiment changé — sinon un rafraîchissement (ex. après chaque
      // lot MCP en mode autonome) faisait "sauter" tout le canvas à chaque
      // fois, même quand rien de nouveau n'était arrivé (bug remonté par Liam
      // le 2026-07-21 : "le canvas se remet à sa forme initiale").
      const ids = (x: BrainGraph) => x.nodes.map((n) => n.id).sort().join("\n");
      if (!prev || ids(prev) !== ids(g)) setRevealKey((k) => k + 1);
      return g;
    });
  }

  // Après une restauration manuelle (Réglages → Compte) : les données restaurées
  // sont un vrai cerveau (fin de la démo), et les espaces ont pu changer aussi.
  async function handleRestored() {
    localStorage.removeItem("lucid.demo");
    setDemoMode(false);
    await refreshGraph();
    listSpaces().then(setSpaces);
    connectorsStatus().then(setConnectors);
  }

  async function handleDeleteNode(nodeId: string) {
    const n = graph?.nodes.find((x) => x.id === nodeId);
    const kids = n ? graph!.nodes.filter((x) => x.parent_id === nodeId).length : 0;
    const detail = kids > 0 ? " et toutes ses sous-pages" : "";
    if (!confirm(`Supprimer « ${n?.label ?? "ce nœud"} »${detail} ? (un snapshot est gardé)`)) return;
    try {
      await deleteNode(nodeId);
      const g = await readBrainGraph();
      if (g) {
        setGraph(g); setRevealKey((k) => k + 1);
        // Le nœud ouvert dans le panneau a pu partir avec le sous-arbre supprimé.
        if (selectedNode && !g.nodes.some((x) => x.id === selectedNode.id)) closeDetail();
      }
      setSpaces(await listSpaces());
    } catch (e) { setError(String(e)); }
  }

  async function handleCreateNote(parentId: string, title: string) {
    const n = await createNoteNode(parentId, title);
    await refreshGraph();
    setSelectedNode(n);
  }

  // ── Propositions MCP : événementiel (watcher Rust sur mcp_pending/) + validation ──
  // Mode autonome (défaut) : toute proposition détectée est auto-acceptée dans
  // la foulée, locale ou distante — le toggle ne regarde jamais l'origine,
  // juste "il y a une proposition en attente" (même tuyau pour les deux MCP).
  // Mode validation manuelle : comportement inchangé, on attend le clic.
  // ponytail: un poll toutes les 2s tournait en continu même sans IA active —
  // remplacé par l'évènement "mcp-proposal-changed" (watcher `notify` côté
  // Rust) + un filet à 30s au cas où l'évènement serait raté (bug remonté par
  // Liam le 2026-07-21 : "les polls en continu consomment trop").
  useEffect(() => {
    if (needsSetup) return;
    let stop = false;
    let running = false; // évite qu'un tick auto-accept en cours (brain.json volumineux
    // → accepter peut prendre plusieurs secondes) chevauche le suivant et écrase son travail.
    // Propositions qui restent en attente d'un tick à l'autre malgré la tentative —
    // au-delà du seuil, un vrai problème (ex. id-parent qui n'existera jamais),
    // pas juste "l'IA n'a pas fini" : on le signale plutôt que de laisser filer en
    // silence (bug remonté par Liam le 2026-07-21 : "44 pages jamais reçues").
    const stuckSince = new Map<string, number>();
    const notifiedStuck = new Set<string>();
    const STUCK_THRESHOLD_MS = 90_000;
    const tick = async () => {
      if (running) return;
      running = true;
      try {
        const manual = await mcpManualValidationEnabled();
        const p = await listMcpProposals();
        if (stop) return;
        // Autonome : jamais affiché (ni bulle ni panneau), juste accepté en silence —
        // `proposals` reste vide pour ne pas déclencher le moindre rendu de plus.
        if (manual) {
          setProposals((prev) => (JSON.stringify(prev) === JSON.stringify(p) ? prev : p));
        } else if (p.length > 0) {
          // Bascule manuel → autonome avec des bulles déjà affichées : les faire
          // disparaître tout de suite plutôt que d'attendre la fin de la résolution.
          setProposals((prev) => (prev.length === 0 ? prev : []));
          // Un seul appel pour tout le lot (pas une boucle par proposition) :
          // un gros lot (arborescence de N pages) ne doit faire qu'un cycle de
          // lecture/écriture de brain.json, pas N (bug remonté par Liam le
          // 2026-07-21 : 17 pages d'un coup rendaient l'app très lente/saccadée).
          const resolvedIds = await resolveAllMcpProposals().catch(() => [] as string[]);
          // Sans ça, l'écriture est réelle (brain.json à jour) mais invisible tant
          // que l'utilisateur ne régénère/rouvre pas — le mode autonome doit se voir.
          if (!stop) { await refreshGraph(); setSpaces(await listSpaces()); }
          if (resolvedIds.length) {
            supabase?.from("mcp_proposals").delete().in("id", resolvedIds).then(() => {}, () => {});
            const n = resolvedIds.length;
            void notify("Archiviste", `${n} page${n > 1 ? "s" : ""} mise${n > 1 ? "s" : ""} à jour dans ton cerveau.`);
          }
          // Suivi des propositions encore en attente malgré cette tentative.
          const resolvedSet = new Set(resolvedIds);
          const now = Date.now();
          const stillPending = new Set(p.filter((x) => !resolvedSet.has(x.id)).map((x) => x.id));
          for (const id of stillPending) if (!stuckSince.has(id)) stuckSince.set(id, now);
          for (const id of [...stuckSince.keys()]) {
            if (!stillPending.has(id)) { stuckSince.delete(id); notifiedStuck.delete(id); continue; }
            if (now - stuckSince.get(id)! > STUCK_THRESHOLD_MS && !notifiedStuck.has(id)) {
              notifiedStuck.add(id);
              void notify(
                "Archiviste — action bloquée",
                "Une proposition de l'IA n'arrive pas à s'appliquer depuis plus d'une minute. Vérifie Lucid.",
              );
            }
          }
        } else {
          setProposals((prev) => (prev.length === 0 ? prev : []));
        }
      } catch { /* app pas prête : silencieux */ }
      finally { running = false; }
    };
    // Débounce du DÉCLENCHEMENT (pas de tick() lui-même) : une IA qui appelle
    // ses outils un par un (ex. réorganiser 17 pages avec 17 move_node séparés
    // dans le temps) faisait repartir un cycle complet à chaque arrivée —
    // regrouper celles qui se suivent de près avant de traiter le lot en un
    // coup (bug remonté par Liam le 2026-07-21 : "réorganisé 17 nœuds, l'app
    // plante beaucoup trop").
    let debounce: ReturnType<typeof setTimeout> | null = null;
    const scheduleTick = () => {
      if (debounce) clearTimeout(debounce);
      debounce = setTimeout(() => { debounce = null; void tick(); }, 1500);
    };
    tick();
    const unlisten = listen("mcp-proposal-changed", scheduleTick);
    const iv = setInterval(tick, 30_000); // filet, pas le mécanisme principal
    return () => {
      stop = true;
      clearInterval(iv);
      if (debounce) clearTimeout(debounce);
      unlisten.then((fn) => fn());
    };
  }, [needsSetup]);

  // ── Propositions MCP distantes (IA via connecteur claude.ai/ChatGPT) :
  //    rapatriées de Supabase vers le circuit local mcp_pending/ (mêmes bulles
  //    fantômes, même validation), puis supprimées de la table. RLS : on ne
  //    voit que les propositions de SES spaces. ──
  // ponytail: pollait la table toutes les 10s en continu, même IA à l'arrêt —
  // remplacé par une souscription Realtime (INSERT sur mcp_proposals, déjà le
  // même mécanisme que la sync cloud dans lib/sync.ts) + un filet à 60s au cas
  // où le websocket serait coupé. Nécessite `docs/supabase-mcp-proposals-realtime.sql`.
  useEffect(() => {
    if (needsSetup || !supabase) return;
    let stop = false;
    let running = false;
    const tick = async () => {
      if (running) return;
      running = true;
      try {
        const { data: sess } = await supabase!.auth.getSession();
        if (!sess.session || stop) return;
        const { data, error } = await supabase!
          .from("mcp_proposals")
          .select("id,action,parent_id,label,content,target_id,new_parent_id,merge_ids,link_target,relation,created_at")
          .limit(20);
        if (error || !data?.length || stop) return;
        for (const p of data) {
          await importMcpProposal({
            id: p.id, action: p.action ?? "create", parent_id: p.parent_id ?? "", label: p.label ?? "",
            content: p.content ?? "", target_id: p.target_id ?? "", new_parent_id: p.new_parent_id ?? "",
            merge_ids: p.merge_ids ?? [], link_target: p.link_target ?? "", relation: p.relation ?? "",
            created_at: p.created_at,
          });
          // La ligne reste 10 min dans la table : l'IA distante peut chaîner des
          // sous-pages (l'edge function vérifie que le parent existe encore).
          // L'import est idempotent → la relire aux ticks suivants est sans effet,
          // et la résolution (accepter/refuser) supprime la ligne immédiatement.
          const age = Date.now() - new Date(p.created_at).getTime();
          if (age > 10 * 60_000) {
            await supabase!.from("mcp_proposals").delete().eq("id", p.id);
          }
        }
      } catch { /* hors-ligne ou table absente : silencieux */ }
      finally { running = false; }
    };
    // Même regroupement que le poll local : une IA qui pose ses 17 outils un
    // par un dans le temps déclenchait 17 imports (et donc 17 résolutions)
    // séparés au lieu d'un lot groupé.
    let debounce: ReturnType<typeof setTimeout> | null = null;
    const scheduleTick = () => {
      if (debounce) clearTimeout(debounce);
      debounce = setTimeout(() => { debounce = null; void tick(); }, 1500);
    };
    tick();
    const channel = supabase
      .channel("mcp-proposals-watch")
      .on("postgres_changes", { event: "INSERT", schema: "public", table: "mcp_proposals" }, scheduleTick)
      .subscribe();
    const iv = setInterval(tick, 60_000); // filet, pas le mécanisme principal
    return () => {
      stop = true;
      clearInterval(iv);
      if (debounce) clearTimeout(debounce);
      void channel.unsubscribe();
    };
  }, [needsSetup]);

  // ── Espace "MCP personnel" : tenu à jour indépendamment de la sync cloud
  //    complète (désactivée actuellement, cf. plus haut) ──
  // Sans ça, l'instantané publié pour les IA distantes peut dater de la
  // dernière fois où `syncNow()` a tourné (rare) — une IA (ChatGPT, claude.ai)
  // se retrouve alors à proposer des pages sous un id-parent qui a changé ou
  // disparu depuis côté local, et ces propositions ne se résolvent JAMAIS
  // (bug remonté par Liam le 2026-07-21 : "44 pages créées, jamais reçues").
  useEffect(() => {
    if (needsSetup || !supabase) return;
    let lastRun = 0;
    const REFRESH_THROTTLE_MS = 30_000; // pas plus d'un envoi Supabase toutes les 30s
    const refresh = () => {
      const now = Date.now();
      if (now - lastRun < REFRESH_THROTTLE_MS) return;
      lastRun = now;
      void ensurePersonalMcpSpace().catch((e) => console.warn("mcp personnel:", e));
    };
    refresh();
    const unlisten = listen("user-data-changed", refresh);
    return () => { unlisten.then((fn) => fn()); };
  }, [needsSetup]);

  async function handleProposal(id: string, accept: boolean) {
    try {
      const ids = await resolveMcpProposal(id, accept);
      // Purge les lignes Supabase de toute la chaîne résolue (si distante) :
      // sinon le poll de rapatriement les ré-importerait en bulles zombies.
      supabase?.from("mcp_proposals").delete().in("id", ids).then(() => {}, () => {});
      setProposals(await listMcpProposals());
      // L'acceptation peut enrôler le nœud dans des spaces (côté Rust) →
      // recharger les deux, sinon la vue filtre avec l'ancien node_ids.
      if (accept) { await refreshGraph(); setSpaces(await listSpaces()); }
      showToast(accept ? `${ids.length > 1 ? `${ids.length} propositions acceptées` : "Proposition acceptée"} ✓` : "Proposition refusée");
    } catch (e) { showToast(String(e)); }
  }

  async function handleAllProposals(accept: boolean) {
    // Accepter : un seul cycle lecture/écriture pour tout le lot (pas une
    // boucle par proposition, cf. resolveAllMcpProposals). Refuser reste une
    // boucle : ça ne fait que retirer de petits fichiers, rien à batcher.
    const resolved = accept
      ? await resolveAllMcpProposals().catch(() => [] as string[])
      : await (async () => {
          const ids: string[] = [];
          for (const p of proposals) {
            try { ids.push(...await resolveMcpProposal(p.id, false)); } catch { /* déjà traitée via une chaîne parent/enfant */ }
          }
          return ids;
        })();
    // Purge Supabase comme le clic unitaire : sinon le poll de rapatriement
    // ré-importe tout en bulles zombies dans les 10 s.
    if (resolved.length) supabase?.from("mcp_proposals").delete().in("id", resolved).then(() => {}, () => {});
    setProposals(await listMcpProposals());
    if (accept) { await refreshGraph(); setSpaces(await listSpaces()); }
    showToast(accept ? "Toutes les propositions acceptées ✓" : "Propositions refusées");
  }

  function showToast(msg: string, ms = 4000) {
    setToast(msg);
    if (toastTimer.current) clearTimeout(toastTimer.current);
    if (ms > 0) toastTimer.current = setTimeout(() => setToast(null), ms);
  }

  /** Drag & drop OS sur le canvas : import séquentiel (brain.json n'aime pas les écritures parallèles). */
  async function handleImportDrop(paths: string[], parentId: string) {
    const supported = new Set(["pdf", "doc", "docx", "pptx", "xlsx", "rtf", "txt", "md", "markdown", "csv"]);
    const ok = paths.filter((p) => supported.has(p.split(".").pop()?.toLowerCase() ?? ""));
    if (!ok.length) { showToast("Formats supportés : PDF, Word, PowerPoint (.pptx), Excel (.xlsx), RTF, TXT, Markdown, CSV"); return; }
    showToast(`Import de ${ok.length} fichier${ok.length > 1 ? "s" : ""}…`, 0);
    let last: BrainNode | null = null;
    const errs: string[] = [];
    for (const p of ok) {
      try { last = await importFile(p, parentId); } catch (e) { errs.push(String(e)); }
    }
    await refreshGraph();
    if (last) { setSelectedNode(last); setUxFlag("importedFile"); }
    showToast(errs.length
      ? `${ok.length - errs.length} importé(s) · échec : ${errs[0]}`
      : `${ok.length} fichier${ok.length > 1 ? "s importés" : " importé"} ✓`);
  }

  async function handleImportFile(parentId: string) {
    const path = await openFileDialog({
      multiple: false,
      filters: [{ name: "Documents", extensions: ["pdf", "doc", "docx", "pptx", "xlsx", "rtf", "txt", "md", "csv"] }],
    });
    if (typeof path !== "string") return;
    setImporting(true);
    setImportError(null);
    try {
      const n = await importFile(path, parentId);
      await refreshGraph();
      setSelectedNode(n);
      setUxFlag("importedFile");
      setNoteOpen(false);
    } catch (e) {
      setImportError(String(e));
    } finally {
      setImporting(false);
    }
  }

  async function handleMoveNode(nodeId: string, parentId: string) {
    await setNodeParent(nodeId, parentId);
    await refreshGraph();
  }

  async function handleSpaceDelete(id: string) {
    await deleteSpace(id);
    setSpaces((prev) => prev.filter((s) => s.id !== id));
    if (activeSpaceId === id) setActiveSpaceId(null);
  }

  const checklistItems: ChecklistItem[] = useMemo(() => [
    {
      id: "page", label: "Ouvrir une page", done: uxFlags.openedPage,
      hint: "Clique sur une bulle de la carte",
    },
    {
      id: "import", label: "Importer un fichier", done: uxFlags.importedFile,
      hint: "Glisse un PDF sur la carte, ou via +",
      onClick: () => { setNoteTitle(""); setNoteParent(rootId); setImportError(null); setNoteOpen(true); },
    },
    {
      // Le contenu starter contient déjà des [[wikilinks]] : on ne compte que
      // ceux écrits par l'utilisateur (hors nœuds demo-*).
      id: "wikilink", label: "Lier deux pages", done:
        (graph?.nodes ?? []).some((n) => !n.id.startsWith("demo-") && n.content?.includes("[[")),
      hint: "Tape [[ dans une page",
    },
    {
      id: "source", label: "Connecter une source", done: connectors.some((c) => c.connected),
      hint: "Claude Code, Google Drive, Obsidian…", onClick: () => setSettingsOpen(true),
    },
    {
      // Le graphe starter (generated_at "demo") ne compte pas comme cerveau généré.
      id: "brain", label: "Générer mon cerveau", done: !!graph && graph.generated_at !== "demo",
      hint: "Tes vraies données remplacent l'exemple", onClick: () => handleGenerate(),
    },
    {
      id: "ai", label: "Brancher une IA", done: aiConnected,
      hint: "Claude, Cursor, Codex…", onClick: () => setSettingsOpen(true),
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
  ], [connectors, graph, uxFlags, aiConnected, rootId]);
  const checklistDone = checklistItems.every((i) => i.done);

  const activeSpace = spaces.find((s) => s.id === activeSpaceId);
  const displayGraph = graph && activeSpace?.node_ids
    ? filterGraphBySpace(graph, activeSpace.node_ids)
    : graph;

  // Wikilinks [[Page]] → ponts "link" entre bulles. Isolé dans SON PROPRE memo,
  // dépendant UNIQUEMENT de displayGraph (pas de `proposals`) : c'est un scan
  // de TOUT le texte de TOUTES les pages (regex sur potentiellement des Mo de
  // contenu) — le recalculer à chaque poll MCP (toutes les 2s, y compris pendant
  // une rafale d'auto-acceptations) gelait l'app sur un gros cerveau. Il ne
  // recalcule maintenant que quand le cerveau change réellement (bug remonté
  // par Liam le 2026-07-21).
  const wikilinkEdges = useMemo(() => {
    if (!displayGraph) return [];
    const byLabel = new Map(displayGraph.nodes.map((n) => [n.label.toLowerCase(), n.id]));
    const seen = new Set<string>();
    const edges: { source: string; target: string; kind: string; relation: string }[] = [];
    for (const n of displayGraph.nodes) {
      if (!n.content) continue;
      for (const m of n.content.matchAll(/\[\[([^[\]\n]+)\]\]/g)) {
        const tid = byLabel.get(m[1].trim().toLowerCase());
        if (!tid || tid === n.id) continue;
        const key = n.id < tid ? `${n.id}|${tid}` : `${tid}|${n.id}`;
        if (seen.has(key)) continue;
        seen.add(key);
        edges.push({ source: n.id, target: tid, kind: "link", relation: "wikilink" });
      }
    }
    return edges;
  }, [displayGraph]);

  // Graphe affiché sur le canvas : graphe (filtré par space) + bulles fantômes
  // MCP + arêtes wikilinks. Fantômes et liens ne sont jamais persistés.
  // "create" = bulle fantôme ambre (nouveau nœud, comme avant). Les 4 autres
  // actions (update/move/merge/link) ciblent des nœuds RÉELS déjà existants —
  // pas de bulle en plus, juste un badge orange (`pendingAction`) sur ces nœuds.
  // Mode autonome : `proposals` reste toujours vide (cf. le poll plus haut) —
  // aucune bulle/badge ne s'affiche jamais, l'IA écrit sans bruit visuel.
  const graphWithGhosts = useMemo(() => {
    if (!displayGraph) return displayGraph;
    if (proposals.length === 0) return { ...displayGraph, edges: [...displayGraph.edges, ...wikilinkEdges] };
    const createProposals = proposals.filter((p) => p.action === "create");
    const otherProposals = proposals.filter((p) => p.action !== "create");
    const pendingByNode = new Map<string, BrainNode["pendingAction"]>();
    for (const p of otherProposals) {
      const targets = p.action === "merge" ? p.merge_ids : [p.target_id];
      for (const id of targets) if (id) pendingByNode.set(id, p.action as BrainNode["pendingAction"]);
    }
    const baseNodes = pendingByNode.size === 0 ? displayGraph.nodes
      : displayGraph.nodes.map((n) => pendingByNode.has(n.id) ? { ...n, pendingAction: pendingByNode.get(n.id) } : n);
    const nodes = createProposals.length === 0 ? baseNodes : [
      ...baseNodes,
      ...createProposals.map((p): BrainNode => ({
        id: p.id, label: p.label, kind: "pending", weight: 0,
        summary: "Proposition de votre IA — à valider", keywords: [], decisions: [], patterns: [],
        community: 0, parent_id: p.parent_id, synthesized_at: null, content: p.content,
      })),
    ];
    const edges = [
      ...displayGraph.edges,
      ...wikilinkEdges,
      ...createProposals.map((p) => ({ source: p.parent_id, target: p.id, kind: "contains", relation: "contains" })),
    ];
    return { ...displayGraph, nodes, edges };
  }, [displayGraph, proposals, wikilinkEdges]);

  // ── Timeline : bornes temporelles du cerveau (nœuds datés) ──
  const timeRange = useMemo(() => {
    const ds = (graph?.nodes ?? [])
      .filter((n) => n.date)
      .map((n) => new Date(n.date!).getTime());
    if (ds.length < 2) return null;
    const min = Math.min(...ds), max = Math.max(...ds);
    return min < max ? { min, max } : null;
  }, [graph]);

  // Curseur quantisé au jour : pendant le replay (rAF), le graphe filtré n'est
  // recalculé qu'au franchissement d'un jour — pas à chaque frame.
  const cutoffDate = timeCutoff === null ? null : new Date(timeCutoff).toISOString().slice(0, 10);

  // Graphe filtré au curseur : une feuille/note datée n'existe qu'après sa date,
  // un conteneur n'existe que s'il a au moins un descendant visible.
  const timelineGraph = useMemo(() => {
    if (!graphWithGhosts || cutoffDate === null) return graphWithGhosts;
    const cutoff = cutoffDate;
    const byId = new Map(graphWithGhosts.nodes.map((n) => [n.id, n]));
    const kids = new Map<string, string[]>();
    for (const n of graphWithGhosts.nodes) {
      if (!n.parent_id) continue;
      const a = kids.get(n.parent_id);
      if (a) a.push(n.id); else kids.set(n.parent_id, [n.id]);
    }
    const memo = new Map<string, boolean>();
    const vis = (id: string): boolean => {
      const got = memo.get(id);
      if (got !== undefined) return got;
      const n = byId.get(id);
      let v: boolean;
      if (!n || n.kind === "root") v = true;
      else if (n.date) v = n.date <= cutoff;
      else {
        const k = kids.get(id);
        // Sans date : conteneur → suit ses enfants ; feuille/note ancienne → toujours là.
        v = !k || k.length === 0 ? true : k.some(vis);
      }
      memo.set(id, v);
      return v;
    };
    const nodes = graphWithGhosts.nodes.filter((n) => vis(n.id));
    const set = new Set(nodes.map((n) => n.id));
    const edges = graphWithGhosts.edges.filter((e) => set.has(e.source) && set.has(e.target));
    return { ...graphWithGhosts, nodes, edges };
  }, [graphWithGhosts, cutoffDate]);

  if (needsSetup) {
    return (
      <SetupScreen
        onDone={() => setNeedsSetup(false)}
        onSkip={() => { localStorage.setItem("lucid.ai.skipped", "1"); setNeedsSetup(false); }}
      />
    );
  }

  return (
    <div className="relative h-screen overflow-hidden bg-[var(--color-bg)] text-[var(--color-text)]">

      {!booted ? null : !graph && !generating && !scanning ? (
        <GenerateEmpty
          error={error}
          onStartScan={handleStartScan}
          onExploreDemo={handleSeedDemo}
          onOpenSettings={() => setSettingsOpen(true)}
        />
      ) : (
        <>
          {/* ── Canvas principal — monté dès le scan : les bulles apparaissent
                 en direct, avec la même animation de "pop" que le vrai graphe
                 (pas un effet séparé), canvas déjà interactif pendant que ça tourne. ── */}
          {view === "map" && (graph || generating || scanning) && (
            <BrainMap
              graph={
                scanning ? scanGraph :
                (generating && partialGraph) ||
                timelineGraph ||
                { nodes: [], edges: [], markdown: "", report: "", generated_at: "" }
              }
              onSelect={generating || scanning ? () => {} : selectNode}
              selectedId={selectedNode?.id ?? null}
              query={query}
              revealKey={revealKey}
              busy={generating || scanning}
              busyMessage={busyMessage}
              spaces={spaces}
              onAddNodeToSpace={handleAddNodeToSpace}
              onMoveNode={handleMoveNode}
              onDeleteNode={handleDeleteNode}
              onImportFiles={handleImportDrop}
              onBackgroundClick={closeDetail}
              panelOffset={selectedNode && !nodeExpanded ? 480 : 0}
              focus={focus}
            />
          )}
          {view === "folder" && displayGraph && (
            <FolderView
              graph={displayGraph}
              onSelect={selectNode}
              selectedId={selectedNode?.id ?? null}
              query={query}
            />
          )}
          {view === "brain" && graph && (
            <MarkdownView markdown={graph.markdown} onRegenerate={handleGenerate} />
          )}

          {/* ── Overlay progression : scan machine puis 1er cerveau (genesis) —
                 même carte flottante pour les deux, jamais bloquante. En
                 régénération, c'est le root « Lucid » qui pulse et parle. ── */}
          {(scanning || (generating && genesisRun)) && (
            <div className="absolute inset-0 z-20 flex items-end justify-center pb-24 pointer-events-none">
              <div className="pointer-events-auto rounded-2xl border border-[var(--color-border)] bg-[var(--color-surface)] px-6 py-4 shadow-[var(--shadow-float)] min-w-[320px]">
                <GenerateProgress
                  progress={scanning ? scanProgress : progress}
                  label={scanning ? "Lucid scanne ton ordinateur…" : undefined}
                />
                {scanning && <ScanSteps steps={scanSteps} />}
              </div>
            </div>
          )}

          {/* ── Dock de widgets (bord gauche) ── */}
          <div className="absolute left-3 top-1/2 z-30 flex -translate-y-1/2 flex-col gap-2">
            <DockBtn
              active={leftPanel === "spaces"}
              title="Spaces"
              onClick={() => setLeftPanel((p) => (p === "spaces" ? null : "spaces"))}
            >
              <Layers className="size-4" />
            </DockBtn>
            <DockBtn
              active={leftPanel === "assistant"}
              title="Lucid IA"
              onClick={() => setLeftPanel((p) => (p === "assistant" ? null : "assistant"))}
            >
              <MessageCircle className="size-4" />
            </DockBtn>
          </div>

          {/* ── Timeline temporelle — posée juste au-dessus de la barre d'outils ── */}
          {view === "map" && graph && !generating && timeRange && (
            <div className="pointer-events-none absolute inset-x-0 bottom-[4.75rem] z-10 flex justify-center">
              <TimelineBar
                min={timeRange.min}
                max={timeRange.max}
                value={timeCutoff ?? timeRange.max}
                onChange={(v) => setTimeCutoff(v >= timeRange.max ? null : v)}
              />
            </div>
          )}

          {/* ── Panneau outil gauche ── */}
          {leftPanel && (
            <div className="panel absolute bottom-4 left-16 top-4 z-30 flex w-[360px] flex-col overflow-hidden rounded-2xl animate-slideInLeft">
              {leftPanel === "spaces" ? (
                <SpacesPanel
                  spaces={spaces}
                  activeSpaceId={activeSpaceId}
                  onSpaceSelect={setActiveSpaceId}
                  onSpaceCreate={handleSpaceCreate}
                  onSpaceShare={setShareSpace}
                  sharedWithMe={sharedWithMe}
                  onOpenShared={(id) => { setRemoteSpaceId(id); setLeftPanel(null); }}
                  onClose={() => setLeftPanel(null)}
                />
              ) : (
                <AssistantPanel
                  onClose={() => setLeftPanel(null)}
                  activeSpaceId={activeSpaceId}
                  onGraphChange={async () => {
                    await refreshGraph();
                    listSpaces().then(setSpaces); // le space actif vient de gagner des nœuds
                  }}
                />
              )}
            </div>
          )}

          {/* ── HUD (pouls du cerveau) ── */}
          {view === "map" && graph && !generating && (
            <div className="pointer-events-none absolute left-4 top-4 z-10 flex items-center gap-1.5 font-mono text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
              <span className="size-1.5 rounded-full bg-[var(--color-ok)]" />
              {graph.nodes.length} nœuds · {spaces.length} espaces
              {lastSync && <> · sync {relativeDate(lastSync)}</>}
            </div>
          )}

          {/* ── Panneau propositions MCP (validation) ── */}
          {proposals.length > 0 && graph && (
            <div className="panel absolute bottom-6 right-6 z-30 w-[320px] overflow-hidden rounded-2xl">
              <div className="flex items-center justify-between border-b border-[var(--color-border)] px-4 py-2.5">
                <p className="text-sm font-semibold text-[var(--color-text)]">
                  <span className="mr-1.5 inline-block size-2 animate-pulse rounded-full bg-[#e0a33c]" />
                  {proposals.length} proposition{proposals.length > 1 ? "s" : ""} de votre IA
                </p>
                <div className="flex gap-1">
                  <button onClick={() => handleAllProposals(true)}
                    className="rounded-md bg-[var(--color-accent)] px-2 py-1 text-[11px] text-white hover:bg-[var(--color-accent-hover)]">
                    Tout accepter
                  </button>
                  <button onClick={() => handleAllProposals(false)}
                    className="rounded-md px-2 py-1 text-[11px] text-[var(--color-muted)] hover:bg-[var(--color-surface-2)]">
                    Tout refuser
                  </button>
                </div>
              </div>
              <div className="max-h-72 overflow-y-auto">
                {proposals.map((p) => {
                  const labelOf = (id: string) => {
                    if (!id) return id;
                    const n = graphWithGhosts?.nodes.find((n) => n.id === id);
                    return n ? (n.kind === "root" ? "Lucid (racine)" : n.label) : id;
                  };
                  const { title, detail } = describeProposal(p, labelOf);
                  return (
                    <div key={p.id} className="border-b border-[var(--color-border)] px-4 py-2.5 last:border-b-0">
                      <div className="flex items-start justify-between gap-2">
                        <div className="min-w-0">
                          <p className="truncate text-sm text-[var(--color-text)]">
                            {p.action !== "create" && (
                              <span className="mr-1 inline-block size-1.5 rounded-full bg-[#f97316]" />
                            )}
                            {title}
                          </p>
                          {detail && <p className="truncate text-[11px] text-[var(--color-muted)]">{detail}</p>}
                        </div>
                        <div className="flex shrink-0 gap-1">
                          <button onClick={() => handleProposal(p.id, true)} title="Accepter"
                            className="rounded-md border border-[var(--color-border)] px-2 py-0.5 text-sm text-[var(--color-ok,#3fb96b)] hover:bg-[var(--color-surface-2)]">
                            ✓
                          </button>
                          <button onClick={() => handleProposal(p.id, false)} title="Refuser"
                            className="rounded-md border border-[var(--color-border)] px-2 py-0.5 text-sm text-[var(--color-muted)] hover:bg-[var(--color-surface-2)]">
                            ✗
                          </button>
                        </div>
                      </div>
                      {p.content && (
                        <details className="mt-1">
                          <summary className="cursor-pointer text-[11px] text-[var(--color-muted)]">aperçu du contenu</summary>
                          <p className="mt-1 max-h-24 overflow-y-auto whitespace-pre-wrap text-[11px] text-[var(--color-muted)]">
                            {p.content.slice(0, 600)}{p.content.length > 600 ? "…" : ""}
                          </p>
                        </details>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          {/* ── Checklist « Bien démarrer » ── */}
          {view === "map" && graph && !generating && !checklistDismissed && !checklistDone && (
            <div className="pointer-events-none absolute bottom-4 left-4 z-20">
              <StarterChecklist
                items={checklistItems}
                onDismiss={() => {
                  localStorage.setItem("lucid.checklist.dismissed", "1");
                  setChecklistDismissed(true);
                }}
              />
            </div>
          )}

          <UpdateBanner />

          {/* ── Toast (feedback import drag & drop) ── */}
          {toast && (
            <div className="panel pointer-events-none absolute bottom-6 left-1/2 z-30 -translate-x-1/2 rounded-full px-4 py-2 text-sm text-[var(--color-text)]">
              {toast}
            </div>
          )}

          {/* ── Modale nouvelle note ── */}
          {noteOpen && graph && (
            <div className="absolute inset-0 z-40 flex items-center justify-center bg-black/40" onClick={() => setNoteOpen(false)}>
              <div
                onClick={(e) => e.stopPropagation()}
                className="panel w-[340px] rounded-2xl p-4"
              >
                <p className="mb-3 text-sm font-semibold text-[var(--color-text)]">Nouvelle note</p>
                <input
                  autoFocus value={noteTitle} onChange={(e) => setNoteTitle(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && noteTitle.trim()) { handleCreateNote(noteParent || rootId, noteTitle); setNoteOpen(false); }
                    if (e.key === "Escape") { e.stopPropagation(); setNoteOpen(false); }
                  }}
                  placeholder="Titre de la note"
                  className="mb-3 w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-2)] px-3 py-2 text-sm text-[var(--color-text)] outline-none focus:border-[var(--color-accent)]"
                />
                <label className="mb-1 block text-[11px] uppercase tracking-wide text-[var(--color-muted)]">Rattacher à</label>
                <button
                  onClick={() => setParentPickerOpen(true)}
                  className="mb-4 flex w-full items-center justify-between gap-2 rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-2)] px-3 py-2 text-sm text-[var(--color-text)] outline-none hover:border-[var(--color-accent)]"
                >
                  <span className="truncate">
                    {(() => {
                      const p = noteParents.find((n) => n.id === (noteParent || rootId));
                      return p ? (p.kind === "root" ? "Lucid (racine)" : p.label) : "Choisir une page";
                    })()}
                  </span>
                  <Search className="size-3.5 shrink-0 text-[var(--color-muted)]" />
                </button>
                {importError && (
                  <p className="mb-3 text-xs text-[var(--color-danger,#e5484d)]">{importError}</p>
                )}
                <div className="flex items-center justify-between gap-2">
                  <button
                    onClick={() => handleImportFile(noteParent || rootId)}
                    disabled={importing}
                    className="rounded-lg border border-[var(--color-border)] px-3 py-1.5 text-sm text-[var(--color-text)] hover:bg-[var(--color-surface-2)] disabled:opacity-50"
                    title="PDF, Word, PowerPoint (.pptx), Excel (.xlsx), RTF, TXT, Markdown ou CSV — converti en markdown"
                  >
                    {importing ? "Import…" : "Importer un fichier"}
                  </button>
                  <div className="flex gap-2">
                    <button onClick={() => setNoteOpen(false)} className="rounded-lg px-3 py-1.5 text-sm text-[var(--color-muted)] hover:bg-[var(--color-surface-2)]">Annuler</button>
                    <button
                      onClick={() => { if (noteTitle.trim()) { handleCreateNote(noteParent || rootId, noteTitle); setNoteOpen(false); } }}
                      className="rounded-lg bg-[var(--color-accent)] px-3 py-1.5 text-sm text-white hover:bg-[var(--color-accent-hover)]"
                    >
                      Créer
                    </button>
                  </div>
                </div>
              </div>
            </div>
          )}

          {/* ── Picker « Rattacher à » de la modale note ── */}
          {parentPickerOpen && graph && (
            <NodePicker
              title="Rattacher à"
              candidates={noteParents}
              graph={graph}
              onPick={(n) => { setNoteParent(n.id); setParentPickerOpen(false); }}
              onClose={() => setParentPickerOpen(false)}
            />
          )}

          {/* ── Palette de commande ⌘K ── */}
          {paletteOpen && displayGraph && (
            <CommandPalette
              graph={displayGraph}
              query={query}
              onQueryChange={setQuery}
              onSelect={handlePaletteSelect}
              onClose={closePalette}
            />
          )}

          {/* ── Barre flottante bas-centre ── */}
          <div className="pointer-events-none absolute bottom-4 left-1/2 z-10 -translate-x-1/2">
            <div className="panel pointer-events-auto flex items-center gap-1 rounded-2xl px-3 py-2">
              {/* Marque */}
              <div className="mr-1 flex items-center gap-2 pl-1.5 pr-3 border-r border-[var(--color-border)]">
                <span className="size-2 rounded-full bg-[var(--color-accent)] shadow-[0_0_8px_var(--color-accent)]" />
                <span className="font-mono text-xs font-semibold tracking-[0.2em]">LUCID</span>
              </div>

              {/* Modes */}
              <ViewBtn active={view === "map"}    onClick={() => setView("map")}>
                <Network    className="size-4" /> Mind
              </ViewBtn>
              <ViewBtn active={view === "folder"} onClick={() => setView("folder")}>
                <FolderTree className="size-4" /> Dossiers
              </ViewBtn>
              <ViewBtn active={view === "brain"}  onClick={() => setView("brain")}>
                <FileText   className="size-4" /> brain.md
              </ViewBtn>

              {/* Recherche ⌘K */}
              <div className="ml-1 flex items-center pl-3 border-l border-[var(--color-border)]">
                <button
                  onClick={() => { setQuery(""); setPaletteOpen(true); }}
                  className="flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-sm text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)] transition-colors"
                >
                  <Search className="size-4" />
                  <kbd className="rounded border border-[var(--color-border)] px-1 py-0.5 font-mono text-[9px]">⌘K</kbd>
                </button>
              </div>

              {/* Créer — l'action primaire, mise en avant */}
              {graph && (
                <div className="ml-1 flex items-center pl-3 border-l border-[var(--color-border)]">
                  <button
                    onClick={() => { setNoteTitle(""); setNoteParent(rootId); setImportError(null); setNoteOpen(true); }}
                    title="Nouvelle note ou import de fichier"
                    className="flex size-8 items-center justify-center rounded-full bg-[var(--color-accent)] text-white shadow-sm transition-transform hover:scale-105"
                  >
                    <Plus className="size-4" />
                  </button>
                </div>
              )}

            </div>
          </div>

          {/* ── Actions app (haut droite, façon Notion) : régénérer, snapshots,
                 paramètres, thème. Masqué quand le panneau détail est ouvert. ── */}
          {!selectedNode && (
            <div className="absolute right-3 top-3 z-20 flex items-center gap-0.5 rounded-full border border-[var(--color-border)] bg-[var(--color-surface)]/75 px-1.5 py-1 shadow-[var(--shadow-float)] backdrop-blur-md">
              <BetaBadge />
              {graph && (
                <button
                  onClick={() => handleGenerate()}
                  title="Régénérer le cerveau (sync des sources)"
                  className="rounded-full p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)] transition-colors"
                >
                  <RefreshCw className="size-4" />
                </button>
              )}
              <div className="relative">
                <button
                  onClick={handleOpenHistory}
                  title="Historique des snapshots"
                  className={cn(
                    "rounded-full p-1.5 transition-colors",
                    historyOpen
                      ? "bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
                      : "text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]",
                  )}
                >
                  <History className="size-4" />
                </button>
                {historyOpen && (
                  <div className="absolute right-0 top-full mt-2 w-72 rounded-xl border border-[var(--color-border)] bg-[var(--color-surface)] shadow-[var(--shadow-float)] overflow-hidden">
                    <div className="px-3 py-2 border-b border-[var(--color-border)] text-xs font-semibold text-[var(--color-muted)] uppercase tracking-wider">
                      Snapshots
                    </div>
                    {snapshots.length === 0 ? (
                      <p className="px-3 py-4 text-xs text-[var(--color-muted)] text-center">
                        Aucun snapshot — régénère le graphe pour en créer un.
                      </p>
                    ) : (
                      <ul className="max-h-64 overflow-y-auto">
                        {snapshots.map((s) => (
                          <li key={s.id} className="flex items-center justify-between gap-2 px-3 py-2 hover:bg-[var(--color-surface-2)] transition-colors">
                            <div className="min-w-0">
                              <p className="text-xs font-medium text-[var(--color-text)] truncate">
                                {relativeTime(s.created_at)}
                              </p>
                              <p className="text-[10px] text-[var(--color-muted)]">
                                {s.node_count} nœuds · {SNAPSHOT_REASON_LABELS[s.reason] ?? s.reason}
                              </p>
                            </div>
                            <button
                              onClick={() => handleRestore(s.id)}
                              disabled={restoring}
                              title="Restaurer ce snapshot"
                              className="shrink-0 flex items-center gap-1 rounded-lg px-2 py-1 text-[10px] font-medium text-[var(--color-accent)] hover:bg-[var(--color-accent-soft)] transition-colors disabled:opacity-40"
                            >
                              <RotateCcw className="size-3" />
                              Restaurer
                            </button>
                          </li>
                        ))}
                      </ul>
                    )}
                  </div>
                )}
              </div>
              <button
                onClick={() => setSettingsOpen(true)}
                title="Paramètres"
                className="rounded-full p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)] transition-colors"
              >
                <Settings className="size-4" />
              </button>
              <ThemeToggle />
            </div>
          )}

          {/* ── Space partagé avec moi, ouvert en lecture seule ── */}
          {remoteSpaceId && (
            <RemoteSpaceView
              spaceId={remoteSpaceId}
              onClose={() => setRemoteSpaceId(null)}
              onForked={async (proj) => {
                setRemoteSpaceId(null);
                await refreshGraph();
                setFocus({ id: proj.id, k: Date.now() });
                showToast(`« ${proj.label} » copié dans ton cerveau ✓`);
              }}
            />
          )}

          {/* ── Modale Partage de space ── */}
          {shareSpace && graph && (
            <ShareModal
              space={shareSpace}
              subgraph={shareSpace.node_ids ? filterGraphBySpace(graph, shareSpace.node_ids) : graph}
              onClose={() => setShareSpace(null)}
            />
          )}

          {/* ── Modale Paramètres ── */}
          {settingsOpen && (
            <SettingsModal
              connectors={connectors}
              spaces={spaces}
              onRefresh={() => connectorsStatus().then(setConnectors)}
              onSyncDone={() => handleGenerate({ skipSync: true })}
              onClose={() => setSettingsOpen(false)}
              onSpaceCreate={handleSpaceCreate}
              onSpaceRename={handleSpaceRename}
              onSpaceDelete={handleSpaceDelete}
              onRestored={handleRestored}
            />
          )}

          {/* ── Panneau détail (montage unique : panneau ⇄ plein écran) ── */}
          {selectedNode && (
            <div
              className={cn(
                "absolute overflow-hidden border border-[var(--color-border)] bg-[var(--color-surface)] shadow-[var(--shadow-float)]",
                nodeExpanded
                  ? "inset-3 z-40 rounded-2xl"
                  : "bottom-3 right-3 top-3 z-10 w-[480px] rounded-xl animate-slideInRight",
              )}
            >
              <NodeDetail
                node={selectedNode}
                graph={graph}
                onSelect={setSelectedNode}
                onClose={closeDetail}
                expanded={nodeExpanded}
                onExpand={() => setNodeExpanded((v) => !v)}
                onContentSaved={handleContentSaved}
                onCreateNote={handleCreateNote}
                onNodeRenamed={handleNodeRenamed}
              />
            </div>
          )}
        </>
      )}

      {/* Bannière contenu starter : remplacé dès la première source connectée. */}
      {demoMode && (
        <div className="absolute left-1/2 top-4 z-40 flex -translate-x-1/2 items-center gap-3 rounded-full border border-[var(--color-border)] bg-[var(--color-surface)] px-4 py-1.5 shadow-[var(--shadow-float)]">
          <span className="text-xs text-[var(--color-muted)]">Contenu d'exemple — connecte une source pour le remplacer par tes données</span>
          <button
            onClick={() => setSettingsOpen(true)}
            className="rounded-full bg-[var(--color-accent)] px-3 py-1 text-xs font-medium text-white hover:opacity-90"
          >
            Connecter une source
          </button>
        </div>
      )}
      {/* Les Settings doivent rester accessibles PAR-DESSUS l'onboarding (bouton
          Configurer), y compris quand il n'y a pas encore de graphe. */}
      {settingsOpen && !graph && !generating && (
        <SettingsModal
          connectors={connectors}
          spaces={spaces}
          onRefresh={() => connectorsStatus().then(setConnectors)}
          onSyncDone={() => handleGenerate({ skipSync: true })}
          onClose={() => { setSettingsOpen(false); connectorsStatus().then(setConnectors); }}
          onSpaceCreate={handleSpaceCreate}
          onSpaceRename={handleSpaceRename}
          onSpaceDelete={handleSpaceDelete}
              onRestored={handleRestored}
        />
      )}
    </div>
  );
}

function relativeTime(ts: number): string {
  const diff = Math.floor(Date.now() / 1000) - ts;
  if (diff < 60)    return "il y a quelques secondes";
  if (diff < 3600)  return `il y a ${Math.floor(diff / 60)} min`;
  if (diff < 86400) return `il y a ${Math.floor(diff / 3600)} h`;
  return `il y a ${Math.floor(diff / 86400)} j`;
}

function DockBtn({
  active, title, onClick, children,
}: {
  active: boolean;
  title: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      className={cn(
        "panel flex size-[38px] items-center justify-center rounded-xl transition-colors",
        active
          ? "text-[var(--color-accent)] !border-[var(--color-accent)]/40"
          : "text-[var(--color-muted)] hover:text-[var(--color-text)]",
      )}
    >
      {children}
    </button>
  );
}

function ViewBtn({
  active, onClick, children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm transition-colors",
        active
          ? "bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
          : "text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]",
      )}
    >
      {children}
    </button>
  );
}

export default App;
