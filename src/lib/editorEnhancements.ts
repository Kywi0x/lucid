import { Extension } from "@tiptap/core";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";
import type { Node as PmNode } from "@tiptap/pm/model";
import Image from "@tiptap/extension-image";
import { convertFileSrc } from "@tauri-apps/api/core";
import { appDataDir } from "@tauri-apps/api/path";

// ─── Images locales ──────────────────────────────────────────────────────────
// Le markdown stocke `![](assets/img-….png)` (relatif, portable, lisible IA) ;
// seul le rendu DOM passe par le protocole asset de Tauri.

let DATA_DIR = "";
appDataDir().then((d) => { DATA_DIR = d.replace(/\/$/, ""); }).catch(() => {});

/** Image TipTap dont le src relatif `assets/…` est résolu à l'affichage. */
export const LucidImage = Image.extend({
  renderHTML({ HTMLAttributes }) {
    const src = HTMLAttributes.src as string | undefined;
    const mapped = src && src.startsWith("assets/") && DATA_DIR
      ? convertFileSrc(`${DATA_DIR}/${src}`)
      : src;
    return ["img", { ...HTMLAttributes, src: mapped }];
  },
});

/** Décorations markdown « live » : le .md stocké reste du texte pur, seul le
 *  rendu est enrichi. RÈGLE : jamais de syntaxe propriétaire — tout est du
 *  markdown standard (GFM/Obsidian) lisible tel quel par les IA.
 *  - Wikilinks `[[Page]]` → chip cliquable (navigation via onNavigate).
 *  - Transclusion `![[Page]]` → carte d'aperçu de la page (resolveEmbed).
 *  - Surlignage `==texte==` → marqueur (syntaxe Obsidian/extended-md).
 *  - Callouts Obsidian `> [!info] / [!tip] / …` → blockquote coloré avec badge.
 *    `[!type]-` (tiret Obsidian) = replié : clic sur le badge pour plier/déplier,
 *    l'état vit DANS le markdown.
 *  - Blocs ```mermaid → diagramme rendu localement sous le code.
 */

const WIKILINK_RE = /(!?)\[\[([^[\]\n]+)\]\]/g;
const HIGHLIGHT_RE = /==([^=\n]+?)==/g;
const CALLOUT_RE = /^\[!(\w+)\](-?)\s?/;

const CALLOUT_KINDS: Record<string, string> = {
  info: "info", note: "info",
  tip: "tip", success: "tip",
  warning: "warning", attention: "warning",
  danger: "danger", error: "danger", bug: "danger",
  question: "question", faq: "question",
};

// ── Mermaid : rendu paresseux + cache par contenu (le widget est keyé, il
//    n'est pas reconstruit à chaque frappe ailleurs dans le document). ──
const mermaidCache = new Map<string, string>(); // code → svg
let mermaidMod: Promise<typeof import("mermaid")> | null = null;

function renderMermaidInto(el: HTMLElement, code: string) {
  const css = (v: string) => getComputedStyle(document.documentElement).getPropertyValue(v).trim();
  const dark = document.documentElement.dataset.theme === "dark" ||
    (document.documentElement.dataset.theme !== "light" && matchMedia("(prefers-color-scheme: dark)").matches);
  const cacheKey = `${dark}:${code}`;
  const cached = mermaidCache.get(cacheKey);
  if (cached) { el.innerHTML = cached; return; }
  el.textContent = "⏳ rendu du diagramme…";
  mermaidMod ??= import("mermaid");
  mermaidMod
    .then(async (m) => {
      // Thème « base » + variables du design system : le diagramme est un
      // citoyen de l'app, pas un embed étranger.
      m.default.initialize({
        startOnLoad: false,
        securityLevel: "strict",
        theme: "base",
        themeVariables: {
          darkMode: dark,
          background: css("--color-surface"),
          primaryColor: css("--color-accent-soft"),
          primaryTextColor: css("--color-text"),
          primaryBorderColor: css("--color-accent"),
          secondaryColor: css("--color-surface-2"),
          tertiaryColor: css("--color-surface"),
          lineColor: css("--color-muted"),
          textColor: css("--color-text"),
          edgeLabelBackground: css("--color-surface"),
          clusterBkg: css("--color-surface-2"),
          fontFamily: "ui-sans-serif, -apple-system, 'Segoe UI', sans-serif",
          fontSize: "13px",
        },
      });
      const { svg } = await m.default.render(`mm-${Math.abs(hashCode(cacheKey))}`, code);
      mermaidCache.set(cacheKey, svg);
      if (el.isConnected) el.innerHTML = svg;
    })
    .catch((e) => { if (el.isConnected) el.textContent = `Diagramme invalide : ${e.message ?? e}`; });
}

function hashCode(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (Math.imul(h, 31) + s.charCodeAt(i)) | 0;
  return h;
}

export interface EmbedInfo { title: string; excerpt: string; }

function buildDecorations(doc: PmNode, opts: MdDecorationsOptions, sel?: { from: number; to: number }): DecorationSet {
  const decos: Decoration[] = [];
  // Live preview à la Obsidian : la syntaxe se masque quand le curseur est
  // ailleurs, réapparaît dès qu'on entre dans la zone pour éditer.
  const cursorIn = (from: number, to: number) =>
    !!sel && sel.from <= to && sel.to >= from;

  doc.descendants((node, pos) => {
    // ── Wikilinks / transclusions dans les nœuds texte ──
    if (node.isText && node.text) {
      WIKILINK_RE.lastIndex = 0;
      let m: RegExpExecArray | null;
      while ((m = WIKILINK_RE.exec(node.text)) !== null) {
        const label = m[2].trim();
        const from = pos + m.index, to = pos + m.index + m[0].length;
        const editing = cursorIn(from, to);
        if (m[1] === "!") {
          // Transclusion ![[Page]] : source en chip discrète + carte d'aperçu.
          decos.push(Decoration.inline(from, to, { class: "wikilink md-embed-src", "data-wikilink": label }));
          if (!editing) {
            decos.push(Decoration.inline(from, from + 3, { class: "md-syntax" }));
            decos.push(Decoration.inline(to - 2, to, { class: "md-syntax" }));
          }
          const info = opts.resolveEmbed?.(label) ?? null;
          decos.push(Decoration.widget(to, () => embedCard(label, info, opts), {
            side: 1, key: `embed:${label}:${info ? hashCode(info.excerpt) : "x"}`,
          }));
        } else {
          decos.push(Decoration.inline(from, to, { class: "wikilink", "data-wikilink": label }));
          if (!editing) {
            decos.push(Decoration.inline(from, from + 2, { class: "md-syntax" }));
            decos.push(Decoration.inline(to - 2, to, { class: "md-syntax" }));
          }
        }
      }
      // ── Surlignage ==texte== ──
      HIGHLIGHT_RE.lastIndex = 0;
      while ((m = HIGHLIGHT_RE.exec(node.text)) !== null) {
        const from = pos + m.index, to = pos + m.index + m[0].length;
        const editing = cursorIn(from, to);
        decos.push(Decoration.inline(from, to, { class: "md-hl" }));
        decos.push(Decoration.inline(from, from + 2, { class: editing ? "md-hl-marker" : "md-syntax" }));
        decos.push(Decoration.inline(to - 2, to, { class: editing ? "md-hl-marker" : "md-syntax" }));
      }
    }

    // ── Callouts : blockquote dont le texte commence par [!type] ──
    if (node.type.name === "blockquote") {
      const first = node.firstChild;
      const m = first?.textContent ? CALLOUT_RE.exec(first.textContent) : null;
      const kind = m ? CALLOUT_KINDS[m[1].toLowerCase()] : undefined;
      if (m && kind) {
        const folded = m[2] === "-";
        decos.push(Decoration.node(pos, pos + node.nodeSize, {
          class: `callout callout-${kind}${folded ? " callout-folded" : ""}`,
        }));
        const markerFrom = pos + 2;
        const rawTo = markerFrom + m[1].length + 3 + m[2].length; // [!type](-)
        if (cursorIn(markerFrom, markerFrom + m[0].length)) {
          // Édition du marqueur : syntaxe brute visible (badge mono cliquable).
          decos.push(Decoration.inline(markerFrom, rawTo, { class: "callout-marker" }));
        } else {
          // Lecture : syntaxe masquée, badge propre (« Note ▾ ») cliquable.
          decos.push(Decoration.inline(markerFrom, markerFrom + m[0].length, { class: "md-syntax" }));
          const word = m[1][0].toUpperCase() + m[1].slice(1).toLowerCase();
          decos.push(Decoration.widget(markerFrom, () => {
            const b = document.createElement("span");
            b.className = "callout-badge";
            b.contentEditable = "false";
            b.textContent = word;
            return b;
          }, { side: 0, key: `co:${m[1]}:${folded}` }));
        }
      }
    }

    // ── Mermaid : diagramme rendu sous le bloc de code ──
    if (node.type.name === "codeBlock" && node.attrs.language === "mermaid") {
      const code = node.textContent.trim();
      if (code) {
        decos.push(Decoration.widget(pos + node.nodeSize, () => {
          const el = document.createElement("div");
          el.className = "mermaid-preview";
          el.contentEditable = "false";
          renderMermaidInto(el, code);
          return el;
        }, { side: 1, key: `mermaid:${hashCode(code)}` }));
      }
    }
  });

  return DecorationSet.create(doc, decos);
}

/** Carte d'aperçu d'une transclusion (DOM pur — pas de React dans un widget). */
function embedCard(label: string, info: EmbedInfo | null, opts: MdDecorationsOptions): HTMLElement {
  const card = document.createElement("div");
  card.className = "md-embed";
  card.contentEditable = "false";
  const title = document.createElement("div");
  title.className = "md-embed-title";
  title.textContent = info ? info.title : `${label} — page introuvable`;
  card.appendChild(title);
  if (info?.excerpt) {
    const body = document.createElement("div");
    body.className = "md-embed-body";
    body.textContent = info.excerpt;
    card.appendChild(body);
  }
  card.addEventListener("mousedown", (e) => {
    e.preventDefault();
    if (info) opts.onNavigate?.(label);
  });
  return card;
}

export interface MdDecorationsOptions {
  /** Clic sur un wikilink → navigation vers la page portant ce label. */
  onNavigate?: (label: string) => void;
  /** Transclusion ![[Page]] : titre + extrait de la page (null = introuvable). */
  resolveEmbed?: (label: string) => EmbedInfo | null;
}

export const MdDecorations = Extension.create<MdDecorationsOptions>({
  name: "mdDecorations",

  addOptions() {
    return { onNavigate: undefined };
  },

  addProseMirrorPlugins() {
    const opts = this.options;
    return [
      new Plugin({
        key: new PluginKey("mdDecorations"),
        state: {
          init: (_, { doc, selection }) => buildDecorations(doc, opts, selection),
          apply: (tr, old) =>
            tr.docChanged || tr.selectionSet
              ? buildDecorations(tr.doc, opts, tr.selection)
              : old,
        },
        props: {
          decorations(state) { return this.getState(state); },
          handleDOMEvents: {
            // mousedown (et non click) : évite que la sélection de l'éditeur avale l'événement.
            mousedown(view, event) {
              const target = event.target as HTMLElement;

              // Badge d'un callout → plier/déplier : on ajoute/retire le tiret
              // Obsidian ([!note]- = replié). L'état vit DANS le markdown.
              const marker = target.closest?.(".callout-marker, .callout-badge");
              if (marker) {
                const posInfo = view.posAtDOM(marker, 0);
                const $pos = view.state.doc.resolve(posInfo);
                // Remonte au blockquote englobant.
                for (let d = $pos.depth; d > 0; d--) {
                  const n = $pos.node(d);
                  if (n.type.name === "blockquote") {
                    const bqPos = $pos.before(d);
                    const m = n.firstChild?.textContent ? CALLOUT_RE.exec(n.firstChild.textContent) : null;
                    if (!m) break;
                    const dashAt = bqPos + 2 + m[1].length + 3; // juste après « ] »
                    const tr = m[2] === "-"
                      ? view.state.tr.delete(dashAt, dashAt + 1)
                      : view.state.tr.insertText("-", dashAt);
                    view.dispatch(tr);
                    event.preventDefault();
                    return true;
                  }
                }
              }

              const el = target.closest?.(".wikilink");
              const label = el?.getAttribute("data-wikilink");
              if (label && opts.onNavigate) {
                // Clic = navigation (comportement chip) ; pour éditer le texte
                // du lien, on entre au clavier depuis l'extérieur.
                event.preventDefault();
                opts.onNavigate(label);
                return true;
              }
              return false;
            },
          },
        },
      }),
    ];
  },
});
