import { useState, useRef, useEffect } from "react";
import { Plus, X, Type, Tags, Calendar, Hash, CheckSquare, Link as LinkIcon, Check } from "lucide-react";
import type { Prop, PropValue } from "@/lib/frontmatter";
import { type PropType, coerce, typeOf, loadTypeRegistry, saveTypeRegistry } from "@/lib/propertyTypes";
import { cn } from "@/lib/utils";

const TYPE_META: Record<PropType, { label: string; Icon: typeof Type }> = {
  text:     { label: "Texte",         Icon: Type },
  tags:     { label: "Tags",          Icon: Tags },
  date:     { label: "Date",          Icon: Calendar },
  number:   { label: "Nombre",        Icon: Hash },
  checkbox: { label: "Case à cocher", Icon: CheckSquare },
  url:      { label: "Lien",          Icon: LinkIcon },
};
const TYPE_ORDER: PropType[] = ["text", "tags", "date", "number", "checkbox", "url"];

/** Panneau « Propriétés » façon Notion/Obsidian, valeurs stockées en frontmatter du .md.
 *  Contrôlé : l'état des valeurs vit dans NodeDetail ; le TYPE (par nom) vit dans un
 *  registre local (voir propertyTypes.ts). */
export function Properties({ props, onChange }: { props: Prop[]; onChange: (p: Prop[]) => void }) {
  const [reg, setReg] = useState<Record<string, PropType>>(loadTypeRegistry);
  const [adding, setAdding] = useState(false);
  const [newKey, setNewKey] = useState("");

  const setValue = (i: number, value: PropValue) => onChange(props.map((p, j) => (j === i ? { ...p, value } : p)));
  const setKey = (i: number, key: string) => onChange(props.map((p, j) => (j === i ? { ...p, key } : p)));
  const remove = (i: number) => onChange(props.filter((_, j) => j !== i));

  const setType = (i: number, type: PropType) => {
    const next = { ...reg, [props[i].key]: type };
    setReg(next);
    saveTypeRegistry(next);
    setValue(i, coerce(props[i].value, type)); // adapte la valeur au nouveau type
  };

  const add = () => {
    const key = newKey.trim();
    if (key && !props.some((p) => p.key === key)) onChange([...props, { key, value: "" }]);
    setNewKey("");
    setAdding(false);
  };

  return (
    <div className="mb-2 border-b border-[var(--color-border)] pb-2">
      {props.map((p, i) => (
        <PropertyRow
          key={i}
          prop={p}
          type={typeOf(p.key, p.value, reg)}
          onType={(t) => setType(i, t)}
          onKey={(k) => setKey(i, k)}
          onValue={(v) => setValue(i, v)}
          onRemove={() => remove(i)}
        />
      ))}
      {adding ? (
        <div className="flex items-center gap-2 px-2 py-1">
          <Type className="size-3.5 shrink-0 text-[var(--color-muted)]" />
          <input
            autoFocus
            value={newKey}
            onChange={(e) => setNewKey(e.target.value)}
            onBlur={add}
            onKeyDown={(e) => {
              if (e.key === "Enter") add();
              if (e.key === "Escape") { setNewKey(""); setAdding(false); }
            }}
            placeholder="Nom de la propriété"
            className="w-full bg-transparent text-xs text-[var(--color-text)] outline-none placeholder:text-[var(--color-muted)]"
          />
        </div>
      ) : (
        <button
          onClick={() => setAdding(true)}
          className="flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-xs text-[var(--color-muted)] transition-colors hover:bg-[var(--color-surface-2)]/60 hover:text-[var(--color-text)]"
        >
          <Plus className="size-3.5" /> Ajouter une propriété
        </button>
      )}
    </div>
  );
}

function PropertyRow({
  prop, type, onType, onKey, onValue, onRemove,
}: {
  prop: Prop;
  type: PropType;
  onType: (t: PropType) => void;
  onKey: (k: string) => void;
  onValue: (v: PropValue) => void;
  onRemove: () => void;
}) {
  return (
    <div className="group grid grid-cols-[9rem_1fr_auto] items-center gap-2 rounded-md px-2 py-1 transition-colors hover:bg-[var(--color-surface-2)]/50">
      <div className="flex min-w-0 items-center gap-1.5 text-[var(--color-muted)]">
        <TypeMenu type={type} onPick={onType} />
        <input
          value={prop.key}
          onChange={(e) => onKey(e.target.value)}
          className="w-full truncate bg-transparent text-xs outline-none focus:text-[var(--color-text)]"
        />
      </div>
      <ValueEditor type={type} value={prop.value} onChange={onValue} />
      <button
        onClick={onRemove}
        title="Supprimer la propriété"
        className="text-[var(--color-muted)] opacity-0 transition-opacity hover:text-[var(--color-err)] group-hover:opacity-100"
      >
        <X className="size-3.5" />
      </button>
    </div>
  );
}

/** Sélecteur de type : icône cliquable → menu déroulant. */
function TypeMenu({ type, onPick }: { type: PropType; onPick: (t: PropType) => void }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const h = (e: MouseEvent) => { if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false); };
    document.addEventListener("mousedown", h);
    return () => document.removeEventListener("mousedown", h);
  }, [open]);

  const Cur = TYPE_META[type].Icon;
  return (
    <div ref={ref} className="relative shrink-0">
      <button
        onClick={() => setOpen((o) => !o)}
        title={`Type : ${TYPE_META[type].label}`}
        className="rounded p-0.5 transition-colors hover:bg-white/10 hover:text-[var(--color-text)]"
      >
        <Cur className="size-3.5" />
      </button>
      {open && (
        <div className="absolute left-0 top-6 z-20 w-44 overflow-hidden rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] py-1 shadow-lg">
          {TYPE_ORDER.map((t) => {
            const { label, Icon } = TYPE_META[t];
            return (
              <button
                key={t}
                onClick={() => { onPick(t); setOpen(false); }}
                className={cn(
                  "flex w-full items-center gap-2 px-2.5 py-1.5 text-xs transition-colors hover:bg-[var(--color-surface-2)]",
                  t === type ? "text-[var(--color-accent)]" : "text-[var(--color-text)]",
                )}
              >
                <Icon className="size-3.5" /> {label}
                {t === type && <Check className="ml-auto size-3" />}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

const INPUT = "w-full bg-transparent text-xs text-[var(--color-text)] outline-none placeholder:text-[var(--color-muted)]/50";

function ValueEditor({ type, value, onChange }: { type: PropType; value: PropValue; onChange: (v: PropValue) => void }) {
  switch (type) {
    case "tags":
      // défensif : si la valeur est encore scalaire (import), on la découpe pour ne rien perdre.
      return <TagsInput value={Array.isArray(value) ? value : (coerce(value, "tags") as string[])} onChange={onChange} />;
    case "checkbox": {
      const checked = value === "true";
      return (
        <button
          onClick={() => onChange(checked ? "false" : "true")}
          className={cn(
            "flex size-4 items-center justify-center rounded border transition-colors",
            checked ? "border-[var(--color-accent)] bg-[var(--color-accent)] text-white" : "border-[var(--color-border)] hover:border-[var(--color-muted)]",
          )}
        >
          {checked && <Check className="size-3" />}
        </button>
      );
    }
    case "date":
      return <input type="date" value={String(value)} onChange={(e) => onChange(e.target.value)} className={INPUT} />;
    case "number":
      return <input inputMode="decimal" value={String(value)} onChange={(e) => onChange(e.target.value)} placeholder="0" className={INPUT} />;
    case "url":
      return <input type="url" value={String(value)} onChange={(e) => onChange(e.target.value)} placeholder="https://…" className={INPUT} />;
    default:
      return <input value={String(value)} onChange={(e) => onChange(e.target.value)} placeholder="Vide" className={INPUT} />;
  }
}

/** Éditeur de tags (chips) : Entrée ou virgule pour ajouter, Backspace pour retirer le dernier. */
function TagsInput({ value, onChange }: { value: string[]; onChange: (v: string[]) => void }) {
  const [draft, setDraft] = useState("");
  const add = () => {
    const t = draft.trim();
    if (t && !value.includes(t)) onChange([...value, t]);
    setDraft("");
  };
  return (
    <div className="flex flex-wrap items-center gap-1">
      {value.map((t, i) => (
        <span key={i} className="flex items-center gap-1 rounded-full bg-[var(--color-accent)]/15 px-2 py-0.5 text-[11px] text-[var(--color-accent)]">
          {t}
          <button onClick={() => onChange(value.filter((_, j) => j !== i))} className="hover:text-[var(--color-err)]">
            <X className="size-2.5" />
          </button>
        </span>
      ))}
      <input
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={add}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === ",") { e.preventDefault(); add(); }
          if (e.key === "Backspace" && !draft && value.length) onChange(value.slice(0, -1));
        }}
        placeholder={value.length ? "" : "Ajouter…"}
        className="min-w-[60px] flex-1 bg-transparent py-0.5 text-[11px] text-[var(--color-text)] outline-none placeholder:text-[var(--color-muted)]/50"
      />
    </div>
  );
}
