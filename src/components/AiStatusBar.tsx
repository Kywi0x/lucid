import { useEffect, useState } from "react";
import { AlertTriangle } from "lucide-react";
import { aiInfo, type AiInfo } from "@/lib/api";

/** Barre de statut des assistants : modèle actif, fenêtre de contexte, et
 *  avertissement si l'input estimé s'approche de la limite. */
export function AiStatusBar({ input = "" }: { input?: string }) {
  const [info, setInfo] = useState<AiInfo | null>(null);
  useEffect(() => { aiInfo().then(setInfo).catch(() => {}); }, []);
  if (!info) return null;

  // Estimation grossière : ~4 caractères par token (suffisant pour un garde-fou).
  const used = Math.ceil(input.length / 4);
  const high = used > info.context_tokens * 0.8;

  return (
    <div className="flex items-center gap-1.5 px-1 pt-1.5 text-[10px] text-[var(--color-muted)]">
      <span className="truncate">
        Lucid IA · {info.model} · {info.context_tokens.toLocaleString()} tokens
      </span>
      {high && (
        <span className="flex shrink-0 items-center gap-1 text-[#e0a33c]">
          <AlertTriangle className="size-3" /> message long (~{used.toLocaleString()})
        </span>
      )}
    </div>
  );
}
