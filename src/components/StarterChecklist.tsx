import { useState } from "react";
import { Check, ChevronDown, ChevronUp, Rocket, X } from "lucide-react";
import { cn } from "@/lib/utils";

export interface ChecklistItem {
  id: string;
  label: string;
  done: boolean;
  /** Action au clic (ouvre le bon écran). Optionnel : certains items sont juste des astuces. */
  onClick?: () => void;
  hint?: string;
}

/** Widget « Bien démarrer » : guide les premiers pas, se replie, se ferme,
 *  disparaît définitivement une fois tout coché. */
export function StarterChecklist({ items, onDismiss }: { items: ChecklistItem[]; onDismiss: () => void }) {
  const [open, setOpen] = useState(true);
  const done = items.filter((i) => i.done).length;

  return (
    <div className="panel pointer-events-auto w-[260px] overflow-hidden rounded-2xl">
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center gap-2 px-3.5 py-2.5 text-left"
      >
        <Rocket className="size-3.5 shrink-0 text-[var(--color-accent)]" />
        <span className="flex-1 text-xs font-semibold text-[var(--color-text)]">Bien démarrer</span>
        <span className="font-mono text-[10px] text-[var(--color-muted)]">{done}/{items.length}</span>
        {open ? <ChevronDown className="size-3.5 text-[var(--color-muted)]" /> : <ChevronUp className="size-3.5 text-[var(--color-muted)]" />}
      </button>

      {open && (
        <>
          {/* Jauge */}
          <div className="mx-3.5 h-1 overflow-hidden rounded-full bg-[var(--color-surface-2)]">
            <div
              className="h-full rounded-full bg-[var(--color-accent)] transition-all duration-300"
              style={{ width: `${Math.round((done / items.length) * 100)}%` }}
            />
          </div>

          <ul className="px-2 py-2">
            {items.map((i) => (
              <li key={i.id}>
                <button
                  onClick={i.onClick}
                  disabled={i.done || !i.onClick}
                  className={cn(
                    "flex w-full items-start gap-2 rounded-lg px-1.5 py-1.5 text-left transition-colors",
                    !i.done && i.onClick && "hover:bg-[var(--color-surface-2)]",
                  )}
                >
                  <span className={cn(
                    "mt-0.5 flex size-3.5 shrink-0 items-center justify-center rounded-full border",
                    i.done
                      ? "border-[var(--color-accent)] bg-[var(--color-accent)]"
                      : "border-[var(--color-border)]",
                  )}>
                    {i.done && <Check className="size-2.5 text-white" />}
                  </span>
                  <span className="min-w-0">
                    <span className={cn(
                      "block text-xs",
                      i.done ? "text-[var(--color-muted)] line-through" : "text-[var(--color-text)]",
                    )}>
                      {i.label}
                    </span>
                    {!i.done && i.hint && (
                      <span className="block text-[10px] leading-tight text-[var(--color-muted)]">{i.hint}</span>
                    )}
                  </span>
                </button>
              </li>
            ))}
          </ul>

          <button
            onClick={onDismiss}
            className="flex w-full items-center justify-center gap-1 border-t border-[var(--color-border)] px-3 py-1.5 text-[10px] text-[var(--color-muted)] hover:text-[var(--color-text)]"
          >
            <X className="size-3" /> Ne plus afficher
          </button>
        </>
      )}
    </div>
  );
}
