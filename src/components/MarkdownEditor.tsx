import { useEffect, useRef, useState } from "react";
import { useEditor, EditorContent } from "@tiptap/react";
import { BubbleMenu, FloatingMenu } from "@tiptap/react/menus";
import StarterKit from "@tiptap/starter-kit";
import { Table } from "@tiptap/extension-table";
import TableRow from "@tiptap/extension-table-row";
import TableHeader from "@tiptap/extension-table-header";
import TableCell from "@tiptap/extension-table-cell";
import Placeholder from "@tiptap/extension-placeholder";
import { Markdown } from "tiptap-markdown";
import {
  Bold, Italic, Heading2, Heading3, List, ListOrdered,
  Code, Table as TableIcon, RowsIcon, Columns3, Trash2, Plus, FileText, Sparkles, Loader2,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { AiDraft } from "./AiDraft";

interface Props {
  content: string;
  onChange: (markdown: string) => void;
  placeholder?: string;
  /** Déclenché quand l'utilisateur choisit « Page » dans le menu slash (/page). */
  onSlashPage?: () => void;
  /** Génère du contenu markdown selon une consigne (contexte : page + sous-pages si demandé). */
  onGenerate?: (instruction: string, includeChildren: boolean) => Promise<string>;
}

type SlashItem = { key: string; label: string; hint: string; action: "page" | "ia" };

function Btn({
  active, onClick, title, children,
}: {
  active?: boolean;
  onClick: () => void;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onMouseDown={(e) => { e.preventDefault(); onClick(); }}
      title={title}
      className={cn(
        "rounded p-1.5 transition-colors",
        active
          ? "bg-[var(--color-accent)]/20 text-[var(--color-accent)]"
          : "text-[var(--color-muted)] hover:bg-white/10 hover:text-[var(--color-text)]",
      )}
    >
      {children}
    </button>
  );
}

const BUBBLE_CLASS =
  "flex items-center gap-0.5 rounded-xl border border-[var(--color-border)] " +
  "bg-[var(--color-surface)] shadow-lg px-1.5 py-1 backdrop-blur-sm";

const FLOAT_CLASS =
  "flex items-center gap-0.5 rounded-xl border border-[var(--color-border)] " +
  "bg-[var(--color-surface)] shadow-md px-1.5 py-1";

const SLASH_ITEMS: SlashItem[] = [
  { key: "page", label: "Page", hint: "Sous-page vierge rattachée à celle-ci", action: "page" },
  { key: "ia",   label: "IA — rédiger", hint: "Générer du contenu avec l'IA", action: "ia" },
];

export function MarkdownEditor({ content, onChange, placeholder = "Écris quelque chose…", onSlashPage, onGenerate }: Props) {
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  // Poignées « + » façon Notion au survol d'un tableau
  const wrapRef = useRef<HTMLDivElement>(null);
  const hoverTableEl = useRef<HTMLTableElement | null>(null);
  const [tblRect, setTblRect] = useState<{ left: number; top: number; width: number; height: number } | null>(null);

  // Menu slash (/page, /ia)
  const edRef = useRef<import("@tiptap/core").Editor | null>(null);
  const slashRef = useRef<{ len: number; items: SlashItem[]; index: number } | null>(null);
  const cbRef = useRef({ onSlashPage, onGenerate });
  cbRef.current = { onSlashPage, onGenerate };
  const [slash, setSlash] = useState<{ left: number; top: number; items: SlashItem[]; index: number } | null>(null);

  // Popover de consigne pour /ia
  const [prompt, setPrompt] = useState<{ left: number; top: number; pos: number } | null>(null);
  const [promptText, setPromptText] = useState("");
  const [promptChildren, setPromptChildren] = useState(false);
  const [promptLoading, setPromptLoading] = useState(false);
  const [promptError, setPromptError] = useState<string | null>(null);

  function closeSlash() { slashRef.current = null; setSlash(null); }

  function selectSlash(index?: number) {
    const ed = edRef.current, s = slashRef.current;
    if (!ed || !s) return;
    const item = s.items[index ?? s.index];
    if (!item) return;
    const to = ed.state.selection.from, from = to - s.len;
    ed.chain().focus().deleteRange({ from, to }).run();
    const pos = ed.state.selection.from;
    const at = slash ? { left: slash.left, top: slash.top } : { left: 0, top: 0 };
    closeSlash();
    if (item.action === "page") { cbRef.current.onSlashPage?.(); }
    else if (item.action === "ia" && cbRef.current.onGenerate) {
      setPromptText(""); setPromptError(null); setPromptChildren(false);
      setPrompt({ left: at.left, top: at.top, pos });
    }
  }

  function moveSlash(delta: number) {
    const s = slashRef.current;
    if (!s) return;
    const index = (s.index + delta + s.items.length) % s.items.length;
    s.index = index;
    setSlash((v) => v ? { ...v, index } : v);
  }

  function detectSlash(ed: import("@tiptap/core").Editor) {
    const sel = ed.state.selection;
    const wrap = wrapRef.current;
    if (!sel.empty || !wrap) { closeSlash(); return; }
    const start = sel.$from.start();
    const before = ed.state.doc.textBetween(start, sel.from, "\n", "￼");
    const m = /(?:^|\s)\/([a-zA-Z]*)$/.exec(before);
    if (!m) { closeSlash(); return; }
    const query = m[1].toLowerCase();
    let items = SLASH_ITEMS.filter((i) => i.key.startsWith(query));
    if (!cbRef.current.onGenerate) items = items.filter((i) => i.action !== "ia");
    if (!cbRef.current.onSlashPage) items = items.filter((i) => i.action !== "page");
    if (items.length === 0) { closeSlash(); return; }
    slashRef.current = { len: m[1].length + 1, items, index: 0 };
    const c = ed.view.coordsAtPos(sel.from);
    const wr = wrap.getBoundingClientRect();
    setSlash({ left: c.left - wr.left, top: c.bottom - wr.top + 4, items, index: 0 });
  }

  async function runGenerate() {
    const text = promptText.trim();
    const ed = edRef.current;
    if (!text || promptLoading || !ed || !prompt || !cbRef.current.onGenerate) return;
    setPromptLoading(true); setPromptError(null);
    try {
      const md = await cbRef.current.onGenerate(text, promptChildren);
      ed.chain().focus().setTextSelection(prompt.pos).run();
      // @ts-expect-error commande custom AiDraft
      ed.commands.insertAiDraft(md);
      setPrompt(null); setPromptText("");
    } catch (e) {
      setPromptError(String(e));
    } finally {
      setPromptLoading(false);
    }
  }

  const editor = useEditor({
    extensions: [
      StarterKit.configure({ codeBlock: { languageClassPrefix: "language-" } }),
      Table.configure({ resizable: true }),
      TableRow,
      TableHeader,
      TableCell,
      Placeholder.configure({ placeholder }),
      Markdown.configure({ html: false, transformPastedText: true }),
      AiDraft,
    ],
    content,
    editorProps: {
      attributes: {
        class: "editor-content prose-lucid focus:outline-none min-h-[100px] px-1 py-2",
      },
      handleKeyDown(_view, event) {
        if (!slashRef.current) return false;
        if (event.key === "Escape") { closeSlash(); return true; }
        if (event.key === "Enter") { selectSlash(); return true; }
        if (event.key === "ArrowDown") { moveSlash(1); return true; }
        if (event.key === "ArrowUp") { moveSlash(-1); return true; }
        return false;
      },
    },
    onUpdate({ editor }) {
      // @ts-expect-error tiptap-markdown storage type
      onChangeRef.current(editor.storage.markdown.getMarkdown());
      detectSlash(editor);
    },
    onSelectionUpdate({ editor }) { detectSlash(editor); },
  });
  edRef.current = editor;

  useEffect(() => {
    if (!editor) return;
    // @ts-expect-error tiptap-markdown storage type
    const current = editor.storage.markdown.getMarkdown();
    if (current !== content) {
      editor.commands.setContent(content || "", { emitUpdate: false });
    }
  }, [content, editor]);

  if (!editor) return null;

  const PAD = 24; // zone d'accroche autour des bords bas/droit
  function onWrapMove(e: React.MouseEvent) {
    const wrap = wrapRef.current;
    if (!wrap) return;
    const wr = wrap.getBoundingClientRect();
    hoverTableEl.current = null;
    wrap.querySelectorAll("table").forEach((t) => {
      const r = t.getBoundingClientRect();
      if (e.clientX >= r.left - 4 && e.clientX <= r.right + PAD &&
          e.clientY >= r.top - 4 && e.clientY <= r.bottom + PAD) hoverTableEl.current = t as HTMLTableElement;
    });
    const f = hoverTableEl.current as HTMLTableElement | null;
    if (f) {
      const r = f.getBoundingClientRect();
      setTblRect({ left: r.left - wr.left, top: r.top - wr.top, width: r.width, height: r.height });
    } else setTblRect(null);
  }

  // Ajoute une colonne/ligne à la FIN du tableau (cible la dernière cellule via le DOM,
  // indépendamment de la position du curseur).
  function addAtEnd(kind: "col" | "row") {
    const t = hoverTableEl.current;
    if (!t || t.rows.length === 0) return;
    const cell = kind === "col"
      ? t.rows[0].cells[t.rows[0].cells.length - 1]
      : t.rows[t.rows.length - 1].cells[0];
    if (!cell) return;
    const pos = editor!.view.posAtDOM(cell, 0);
    const chain = editor!.chain().focus().setTextSelection(pos + 1);
    (kind === "col" ? chain.addColumnAfter() : chain.addRowAfter()).run();
  }

  return (
    <div ref={wrapRef} className="relative" onMouseMove={onWrapMove} onMouseLeave={() => setTblRect(null)}>
      {/* Toolbar flottante sur sélection de texte */}
      <BubbleMenu editor={editor} className={BUBBLE_CLASS}>
        <Btn active={editor.isActive("bold")} onClick={() => editor.chain().focus().toggleBold().run()} title="Gras">
          <Bold className="size-3.5" />
        </Btn>
        <Btn active={editor.isActive("italic")} onClick={() => editor.chain().focus().toggleItalic().run()} title="Italique">
          <Italic className="size-3.5" />
        </Btn>
        <Btn active={editor.isActive("code")} onClick={() => editor.chain().focus().toggleCode().run()} title="Code">
          <Code className="size-3.5" />
        </Btn>
        <span className="mx-0.5 h-4 w-px bg-[var(--color-border)]" />
        <Btn active={editor.isActive("heading", { level: 2 })} onClick={() => editor.chain().focus().toggleHeading({ level: 2 }).run()} title="Titre H2">
          <Heading2 className="size-3.5" />
        </Btn>
        <Btn active={editor.isActive("heading", { level: 3 })} onClick={() => editor.chain().focus().toggleHeading({ level: 3 }).run()} title="Titre H3">
          <Heading3 className="size-3.5" />
        </Btn>
        {editor.isActive("table") && (
          <>
            <span className="mx-0.5 h-4 w-px bg-[var(--color-border)]" />
            <Btn active={false} onClick={() => editor.chain().focus().addRowAfter().run()} title="Ajouter une ligne">
              <RowsIcon className="size-3.5" />
            </Btn>
            <Btn active={false} onClick={() => editor.chain().focus().addColumnAfter().run()} title="Ajouter une colonne">
              <Columns3 className="size-3.5" />
            </Btn>
            <Btn active={false} onClick={() => editor.chain().focus().deleteTable().run()} title="Supprimer le tableau">
              <Trash2 className="size-3.5" />
            </Btn>
          </>
        )}
      </BubbleMenu>

      {/* Menu flottant sur ligne vide */}
      <FloatingMenu editor={editor} className={FLOAT_CLASS}>
        <Btn active={editor.isActive("heading", { level: 2 })} onClick={() => editor.chain().focus().toggleHeading({ level: 2 }).run()} title="Titre H2">
          <Heading2 className="size-3.5" />
        </Btn>
        <Btn active={editor.isActive("heading", { level: 3 })} onClick={() => editor.chain().focus().toggleHeading({ level: 3 }).run()} title="Titre H3">
          <Heading3 className="size-3.5" />
        </Btn>
        <span className="mx-0.5 h-4 w-px bg-[var(--color-border)]" />
        <Btn active={editor.isActive("bulletList")} onClick={() => editor.chain().focus().toggleBulletList().run()} title="Liste à puces">
          <List className="size-3.5" />
        </Btn>
        <Btn active={editor.isActive("orderedList")} onClick={() => editor.chain().focus().toggleOrderedList().run()} title="Liste numérotée">
          <ListOrdered className="size-3.5" />
        </Btn>
        <span className="mx-0.5 h-4 w-px bg-[var(--color-border)]" />
        <Btn active={false} onClick={() => editor.chain().focus().insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run()} title="Insérer un tableau">
          <TableIcon className="size-3.5" />
        </Btn>
      </FloatingMenu>

      <EditorContent editor={editor} className="text-sm" />

      {/* Menu slash (/page, /ia) */}
      {slash && (
        <div
          style={{ position: "absolute", left: slash.left, top: slash.top, zIndex: 40 }}
          className="min-w-[220px] rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] shadow-[var(--shadow-float)] p-1"
        >
          {slash.items.map((item, i) => (
            <button
              key={item.key}
              type="button"
              onMouseDown={(e) => { e.preventDefault(); selectSlash(i); }}
              className={cn(
                "flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left transition-colors",
                i === slash.index ? "bg-[var(--color-surface-2)]" : "hover:bg-[var(--color-surface-2)]",
              )}
            >
              <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-[var(--color-surface-2)] text-[var(--color-accent)]">
                {item.action === "ia" ? <Sparkles className="size-3.5" /> : <FileText className="size-3.5" />}
              </span>
              <span className="min-w-0">
                <span className="block text-sm text-[var(--color-text)]">{item.label}</span>
                <span className="block text-[11px] text-[var(--color-muted)]">{item.hint}</span>
              </span>
            </button>
          ))}
        </div>
      )}

      {/* Popover de consigne pour /ia */}
      {prompt && (
        <div
          style={{ position: "absolute", left: prompt.left, top: prompt.top, zIndex: 40 }}
          className="w-[320px] rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] shadow-[var(--shadow-float)] p-2.5"
        >
          <div className="mb-2 flex items-center gap-1.5">
            <Sparkles className="size-3.5 text-[var(--color-accent)]" />
            <span className="text-xs font-semibold text-[var(--color-text)]">Rédiger avec l'IA</span>
          </div>
          <textarea
            autoFocus
            value={promptText}
            onChange={(e) => setPromptText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); runGenerate(); }
              else if (e.key === "Escape") { setPrompt(null); }
            }}
            placeholder="Ex : rédige une introduction sur…"
            rows={2}
            className="mb-2 w-full resize-none rounded-md border border-[var(--color-border)] bg-[var(--color-surface-2)] px-2.5 py-1.5 text-xs text-[var(--color-text)] outline-none"
          />
          {promptError && <p className="mb-2 text-[11px] text-red-400">{promptError}</p>}
          <div className="flex items-center justify-between gap-2">
            <button
              type="button"
              onClick={() => setPromptChildren((v) => !v)}
              className={cn(
                "flex items-center gap-1.5 rounded-full border px-2 py-1 text-[10px] transition-colors",
                promptChildren
                  ? "border-[var(--color-accent)] bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
                  : "border-[var(--color-border)] text-[var(--color-muted)] hover:text-[var(--color-text)]",
              )}
            >
              <span className={cn("size-1.5 rounded-full", promptChildren ? "bg-[var(--color-accent)]" : "bg-[var(--color-muted)]")} />
              Sous-pages
            </button>
            <div className="flex items-center gap-1.5">
              <button onClick={() => setPrompt(null)} className="rounded-md px-2 py-1 text-xs text-[var(--color-muted)] hover:bg-[var(--color-surface-2)]">Annuler</button>
              <button
                onClick={runGenerate}
                disabled={promptLoading || !promptText.trim()}
                className="flex items-center gap-1.5 rounded-md bg-[var(--color-accent)] px-2.5 py-1 text-xs font-medium text-white hover:opacity-90 disabled:opacity-40"
              >
                {promptLoading && <Loader2 className="size-3 animate-spin" />}
                Générer
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Poignées « + » au survol du tableau */}
      {tblRect && editor.isEditable && (
        <>
          <button
            type="button"
            title="Ajouter une colonne"
            onMouseDown={(e) => { e.preventDefault(); addAtEnd("col"); }}
            className="absolute z-10 flex items-center justify-center rounded-md border border-[var(--color-border)] bg-[var(--color-surface-2)] text-[var(--color-muted)] hover:bg-[var(--color-accent)] hover:text-white transition-colors"
            style={{ left: tblRect.left + tblRect.width + 3, top: tblRect.top, width: 16, height: tblRect.height }}
          >
            <Plus className="size-3.5" />
          </button>
          <button
            type="button"
            title="Ajouter une ligne"
            onMouseDown={(e) => { e.preventDefault(); addAtEnd("row"); }}
            className="absolute z-10 flex items-center justify-center rounded-md border border-[var(--color-border)] bg-[var(--color-surface-2)] text-[var(--color-muted)] hover:bg-[var(--color-accent)] hover:text-white transition-colors"
            style={{ left: tblRect.left, top: tblRect.top + tblRect.height + 3, width: tblRect.width, height: 16 }}
          >
            <Plus className="size-3.5" />
          </button>
        </>
      )}
    </div>
  );
}
