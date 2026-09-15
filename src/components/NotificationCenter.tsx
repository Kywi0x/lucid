import { useEffect, useRef, useState } from "react";
import { ICONS, CONNECTOR_ICON } from "@/lib/connector-icons";
import type { NotifItem } from "@/lib/notifications";

/** Lamelles de verre posées sur le canvas — pas de panneau : c'est ce qu'on voit
 *  PASSER ENTRE les cartes qui produit l'effet (maquette « l'app vivante »). */

function toneColor(tone: NotifItem["tone"]): string | null {
  return tone === "err" ? "var(--color-err)"
    : tone === "warn" ? "var(--color-warn)"
    : tone === "ok" ? "var(--color-ok)"
    : null;
}

function Ico({ name }: { name: string }) {
  const icon = ICONS[CONNECTOR_ICON[name] ?? name];
  if (!icon) {
    // Lucid lui-même : pas de logo de source, la marque accentuée.
    return <span className="ico" style={{ color: "var(--color-accent)" }}>✦</span>;
  }
  return <span className="ico"><img src={icon.src} alt="" /></span>;
}

function Actions({ actions, onGo }: {
  actions?: NotifItem["actions"];
  onGo: (go: string, done: () => void) => void;
}) {
  const [done, setDone] = useState<string | null>(null);
  if (!actions?.length) return null;
  return (
    <div className="row">
      {actions.map((a) => (
        <button
          key={a.label}
          className={"mini-btn" + (a.primary ? " ok" : "")}
          disabled={done === a.label}
          onClick={(e) => {
            e.stopPropagation();
            if (a.go) onGo(a.go, () => setDone(a.label));
            else setDone(a.label);
          }}
        >
          {done === a.label ? "Fait" : a.label}
        </button>
      ))}
    </div>
  );
}

function Card({ n, onDismiss, onGo }: {
  n: NotifItem;
  onDismiss: (id: string) => void;
  onGo: (go: string, done: () => void) => void;
}) {
  const col = toneColor(n.tone);
  return (
    <div className={"ncard" + (col ? " act" : "")}>
      {col && <span className="bar" style={{ background: col }} />}
      <Ico name={n.icon} />
      <div className="body">
        <div className="top">
          <span className="ttl">{n.title}</span>
          {n.group && <span className="more">{n.group.length}</span>}
          {n.when && <span className="when">{n.when}</span>}
        </div>
        {n.text && <div className="txt">{n.text}</div>}
        <Actions actions={n.actions} onGo={onGo} />
      </div>
      <button
        className="x"
        aria-label="Effacer"
        onClick={(e) => { e.stopPropagation(); onDismiss(n.id); }}
      >
        ✕
      </button>
    </div>
  );
}

export function NotificationCenter({
  items, onDismiss, onClear, onGo, onClose, footer,
}: {
  items: NotifItem[];
  onDismiss: (id: string) => void;
  onClear: () => void;
  onGo: (go: string, done: () => void) => void;
  onClose: () => void;
  footer: string;
}) {
  const [openStacks, setOpenStacks] = useState<Set<string>>(new Set());
  // Carte en cours de fermeture : elle glisse et son emplacement s'effondre —
  // sans ça la liste sursaute et on perd de vue où on en était.
  const [leaving, setLeaving] = useState<Set<string>>(new Set());
  const listRef = useRef<HTMLDivElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  function dismiss(id: string) {
    setLeaving((s) => new Set(s).add(id));
    setTimeout(() => {
      onDismiss(id);
      setLeaving((s) => { const n = new Set(s); n.delete(id); return n; });
    }, 300);
  }

  // Un clic à côté ferme, comme les modales.
  useEffect(() => {
    function onClick(e: MouseEvent) {
      const t = e.target as HTMLElement | null;
      if (!t) return;
      if (rootRef.current?.contains(t) || t.closest("#ncBtn")) return;
      onClose();
    }
    document.addEventListener("click", onClick);
    return () => document.removeEventListener("click", onClick);
  }, [onClose]);

  // Le reflet sur les lamelles : une seule écoute déléguée (les cartes sont
  // recréées à chaque rendu). Inerte en sombre — --lg-spec y vaut transparent.
  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    let raf = 0;
    function move(e: PointerEvent) {
      if (raf) return;
      raf = requestAnimationFrame(() => {
        const c = (e.target as HTMLElement)?.closest?.(".ncard") as HTMLElement | null;
        if (c) {
          const r = c.getBoundingClientRect();
          c.style.setProperty("--mx", `${((e.clientX - r.left) / r.width) * 100}%`);
          c.style.setProperty("--my", `${((e.clientY - r.top) / r.height) * 100}%`);
        }
        raf = 0;
      });
    }
    list.addEventListener("pointermove", move);
    return () => list.removeEventListener("pointermove", move);
  }, []);

  const pending = items.filter((n) => n.sec === "À traiter").length;
  let section: string | null = null;

  return (
    <div className="nc" ref={rootRef} role="dialog" aria-label="Notifications">
      <div className="nc-head">
        <span className="t">Notifications</span>
        {items.length > pending && (
          <button className="lnk" onClick={onClear}>Effacer l'historique</button>
        )}
      </div>

      <div className="nc-list" ref={listRef}>
        {items.length === 0 ? (
          <div className="nc-empty">
            <span className="big">✦</span>
            <span className="t2">Rien à signaler</span>
            <span className="s2">
              Lucid continue de lire tes sources en fond. Il te préviendra quand quelque chose
              arrive — ou quand quelque chose échoue.
            </span>
          </div>
        ) : (
          items.map((n) => {
            const head = n.sec !== section ? (section = n.sec) : null;
            const open = openStacks.has(n.id);
            return (
              <div key={n.id} style={{ display: "contents" }}>
                {head && <div className="nc-sec">{head}</div>}
                <div className="nc-slot" data-out={leaving.has(n.id) || undefined}>
                  {n.group ? (
                    <div
                      className="stack"
                      data-open={open}
                      onClick={(e) => {
                        const t = e.target as HTMLElement;
                        if (t.closest(".x") || t.closest(".mini-btn") || t.closest(".stack-items")) return;
                        setOpenStacks((s) => {
                          const next = new Set(s);
                          next.has(n.id) ? next.delete(n.id) : next.add(n.id);
                          return next;
                        });
                      }}
                    >
                      <span className="ghost-l g2" />
                      <span className="ghost-l g1" />
                      <Card n={n} onDismiss={dismiss} onGo={onGo} />
                      <div className="stack-items">
                        <div>
                          <div className="inner">
                            {n.group.map((g, i) => (
                              <div
                                key={i}
                                className="ncard"
                                onClick={() => g.go && onGo(g.go, () => {})}
                              >
                                <Ico name={g.icon} />
                                <div className="body">
                                  <div className="top"><span className="ttl">{g.title}</span></div>
                                  <div className="txt">{g.text}</div>
                                </div>
                              </div>
                            ))}
                          </div>
                        </div>
                      </div>
                    </div>
                  ) : (
                    <Card n={n} onDismiss={dismiss} onGo={onGo} />
                  )}
                </div>
              </div>
            );
          })
        )}
      </div>

      <div className="nc-foot">
        <span className="d" />
        <span>{footer}</span>
      </div>
    </div>
  );
}
