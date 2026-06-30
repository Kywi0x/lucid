import { useEffect, useRef } from "react";
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
  Code, Table as TableIcon, RowsIcon, Trash2,
} from "lucide-react";
import { cn } from "@/lib/utils";

interface Props {
  content: string;
  onChange: (markdown: string) => void;
  placeholder?: string;
}

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

export function MarkdownEditor({ content, onChange, placeholder = "Écris quelque chose…" }: Props) {
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  const editor = useEditor({
    extensions: [
      StarterKit.configure({ codeBlock: { languageClassPrefix: "language-" } }),
      Table.configure({ resizable: true }),
      TableRow,
      TableHeader,
      TableCell,
      Placeholder.configure({ placeholder }),
      Markdown.configure({ html: false, transformPastedText: true }),
    ],
    content,
    editorProps: {
      attributes: {
        class: "editor-content prose-lucid focus:outline-none min-h-[100px] px-1 py-2",
      },
    },
    onUpdate({ editor }) {
      // @ts-expect-error tiptap-markdown storage type
      onChangeRef.current(editor.storage.markdown.getMarkdown());
    },
  });

  useEffect(() => {
    if (!editor) return;
    // @ts-expect-error tiptap-markdown storage type
    const current = editor.storage.markdown.getMarkdown();
    if (current !== content) {
      editor.commands.setContent(content || "", { emitUpdate: false });
    }
  }, [content, editor]);

  if (!editor) return null;

  return (
    <div className="relative">
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
    </div>
  );
}
