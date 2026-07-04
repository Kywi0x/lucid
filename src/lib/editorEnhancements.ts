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
 *  rendu est enrichi.
 *  - Wikilinks `[[Page]]` → chip cliquable (navigation via onNavigate).
 *  - Callouts Obsidian `> [!info] / [!tip] / [!warning] / [!danger] / [!question]`
 *    → blockquote coloré avec badge.
 */

const WIKILINK_RE = /\[\[([^[\]\n]+)\]\]/g;
const CALLOUT_RE = /^\[!(\w+)\]\s?/;

const CALLOUT_KINDS: Record<string, string> = {
  info: "info", note: "info",
  tip: "tip", success: "tip",
  warning: "warning", attention: "warning",
  danger: "danger", error: "danger", bug: "danger",
  question: "question", faq: "question",
};

function buildDecorations(doc: PmNode): DecorationSet {
  const decos: Decoration[] = [];

  doc.descendants((node, pos) => {
    // ── Wikilinks dans les nœuds texte ──
    if (node.isText && node.text) {
      WIKILINK_RE.lastIndex = 0;
      let m: RegExpExecArray | null;
      while ((m = WIKILINK_RE.exec(node.text)) !== null) {
        decos.push(Decoration.inline(pos + m.index, pos + m.index + m[0].length, {
          class: "wikilink",
          "data-wikilink": m[1].trim(),
        }));
      }
    }

    // ── Callouts : blockquote dont le texte commence par [!type] ──
    if (node.type.name === "blockquote") {
      const first = node.firstChild;
      const m = first?.textContent ? CALLOUT_RE.exec(first.textContent) : null;
      const kind = m ? CALLOUT_KINDS[m[1].toLowerCase()] : undefined;
      if (m && kind) {
        decos.push(Decoration.node(pos, pos + node.nodeSize, {
          class: `callout callout-${kind}`,
        }));
        // Badge sur le marqueur [!type] (début du premier paragraphe)
        decos.push(Decoration.inline(pos + 2, pos + 2 + m[1].length + 3, {
          class: "callout-marker",
        }));
      }
    }
  });

  return DecorationSet.create(doc, decos);
}

export interface MdDecorationsOptions {
  /** Clic sur un wikilink → navigation vers la page portant ce label. */
  onNavigate?: (label: string) => void;
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
          init: (_, { doc }) => buildDecorations(doc),
          apply: (tr, old) => (tr.docChanged ? buildDecorations(tr.doc) : old),
        },
        props: {
          decorations(state) { return this.getState(state); },
          handleDOMEvents: {
            // mousedown (et non click) : évite que la sélection de l'éditeur avale l'événement.
            mousedown(_view, event) {
              const el = (event.target as HTMLElement).closest?.(".wikilink");
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
