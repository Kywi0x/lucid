import { Node, mergeAttributes } from "@tiptap/core";
import { ReactNodeViewRenderer, NodeViewWrapper, type NodeViewProps } from "@tiptap/react";
import ReactMarkdown from "react-markdown";
import { Sparkles } from "lucide-react";

// Encart « Proposition IA » : contenu généré, non éditable, avec Accepter / Refuser.
// Non persisté (sérialisation markdown no-op) → un brouillon en attente n'est jamais sauvegardé.

function AiDraftView({ node, editor, getPos }: NodeViewProps) {
  const md = (node.attrs.markdown as string) ?? "";

  const accept = () => {
    const pos = getPos();
    if (typeof pos !== "number") return;
    // insertContentAt est patché par tiptap-markdown → le markdown est parsé en contenu formaté.
    editor.chain().focus().deleteRange({ from: pos, to: pos + node.nodeSize }).insertContentAt(pos, md).run();
  };
  const refuse = () => {
    const pos = getPos();
    if (typeof pos !== "number") return;
    editor.chain().focus().deleteRange({ from: pos, to: pos + node.nodeSize }).run();
  };

  return (
    <NodeViewWrapper contentEditable={false} className="my-2">
      <div className="rounded-lg border border-[var(--color-accent)]/40 bg-[var(--color-accent-soft)] px-3 py-2.5">
        <div className="mb-1.5 flex items-center gap-1.5">
          <Sparkles className="size-3 text-[var(--color-accent)]" />
          <span className="text-[10px] font-semibold uppercase tracking-wide text-[var(--color-accent)]">Proposition IA</span>
        </div>
        <div className="prose-lucid text-sm">
          <ReactMarkdown>{md}</ReactMarkdown>
        </div>
        <div className="mt-2.5 flex justify-end gap-2">
          <button
            onClick={refuse}
            className="rounded-md px-2.5 py-1 text-xs text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] transition-colors"
          >
            Refuser
          </button>
          <button
            onClick={accept}
            className="rounded-md bg-[var(--color-accent)] px-2.5 py-1 text-xs font-medium text-white hover:opacity-90 transition-opacity"
          >
            Accepter
          </button>
        </div>
      </div>
    </NodeViewWrapper>
  );
}

export const AiDraft = Node.create({
  name: "aiDraft",
  group: "block",
  atom: true,
  selectable: false,
  draggable: false,

  addAttributes() {
    return { markdown: { default: "" } };
  },

  parseHTML() {
    return [{ tag: "div[data-ai-draft]" }];
  },
  renderHTML({ HTMLAttributes }) {
    return ["div", mergeAttributes(HTMLAttributes, { "data-ai-draft": "" })];
  },

  // tiptap-markdown : ne rien écrire → le brouillon n'apparaît pas dans le markdown sauvegardé.
  addStorage() {
    return { markdown: { serialize: () => {}, parse: {} } };
  },

  addNodeView() {
    return ReactNodeViewRenderer(AiDraftView);
  },

  addCommands() {
    return {
      insertAiDraft: (markdown: string) => ({ state, dispatch }: any) => {
        const n = state.schema.nodes.aiDraft.create({ markdown });
        if (dispatch) dispatch(state.tr.replaceSelectionWith(n).scrollIntoView());
        return true;
      },
    } as any;
  },
});
