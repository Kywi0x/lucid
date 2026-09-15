import { useRef } from "react";
import { LayoutGrid, Network, Plus } from "lucide-react";
import type { ConnectorStatus } from "@/lib/types";
import { ICONS, iconOf } from "@/lib/connector-icons";
import { connState, connMeta } from "@/components/ConnectorTile";

/**
 * Dock — les sources branchées, en bas au centre (maquette « l'app vivante »).
 * Une tuile par source ajoutée, son état en pastille, puis la tuile Launchpad.
 *
 * Les deux dernières tuiles (vue mosaïque, nouvelle page) ne viennent pas de la
 * maquette : elles portaient la barre du bas qu'on remplace ici, et personne
 * n'a demandé de retirer ces deux gestes. Elles sont dans la même matière, à
 * part derrière un séparateur.
 */
export function Dock({
  connectors, onOpenLaunchpad, onOpenConnector, view, onToggleView, onCreate,
}: {
  connectors: ConnectorStatus[];
  onOpenLaunchpad: () => void;
  onOpenConnector: (id: string) => void;
  view: "map" | "mosaic";
  onToggleView: () => void;
  onCreate?: () => void;
}) {
  const dockRef = useRef<HTMLDivElement>(null);
  const tipRef = useRef<HTMLDivElement>(null);
  const rafRef = useRef(0);

  const added = connectors.filter((c) => c.connected || c.reconnect_reason);

  /** Loupe du dock : l'échelle suit la distance au curseur (gaussienne), comme
   *  le dock macOS. Écrit en style direct dans une frame — un state ici
   *  rerendrait tout le dock à chaque pixel. */
  function onMove(e: React.PointerEvent) {
    if (rafRef.current) return;
    const x = e.clientX;
    const target = (e.target as HTMLElement).closest(".tile") as HTMLElement | null;
    rafRef.current = requestAnimationFrame(() => {
      const dock = dockRef.current;
      if (dock) {
        for (const t of Array.from(dock.querySelectorAll<HTMLElement>(".tile"))) {
          const r = t.getBoundingClientRect();
          const d = Math.abs(x - (r.left + r.width / 2));
          const f = Math.exp(-((d / 62) ** 2));
          t.style.transform = `scale(${1 + 0.28 * f}) translateY(${-9 * f}px)`;
        }
      }
      const tip = tipRef.current;
      if (tip && dock) {
        if (target) {
          const id = target.dataset.id;
          const c = added.find((x) => x.id === id);
          tip.querySelector(".nm")!.textContent = c ? c.name : (target.dataset.tip ?? "Launchpad");
          tip.querySelector(".st")!.textContent = c ? `· ${connMeta(c)}` : (target.dataset.sub ?? "· toutes les sources");
          const tr = target.getBoundingClientRect();
          const wr = dock.parentElement!.getBoundingClientRect();
          tip.style.left = `${tr.left - wr.left + tr.width / 2}px`;
          tip.style.transform = "translateX(-50%)";
          tip.dataset.show = "true";
        } else {
          tip.dataset.show = "false";
        }
      }
      rafRef.current = 0;
    });
  }

  function onLeave() {
    const dock = dockRef.current;
    if (dock) for (const t of Array.from(dock.querySelectorAll<HTMLElement>(".tile"))) t.style.transform = "";
    if (tipRef.current) tipRef.current.dataset.show = "false";
  }

  return (
    <div className="dock-wrap">
      <div className="tip lg" ref={tipRef}>
        <span className="nm" /> <span className="st" />
      </div>
      <div className="dock lg" ref={dockRef} onPointerMove={onMove} onPointerLeave={onLeave}>
        {added.map((c) => (
          <button
            key={c.id}
            className={`tile ${iconOf(c.id).plate ? "plate" : "bleed"}`}
            data-id={c.id}
            aria-label={c.name}
            onClick={() => onOpenConnector(c.id)}
          >
            <img src={iconOf(c.id).src} alt="" />
            <span className={`dot ${connState(c)}`} />
          </button>
        ))}

        <span className="dock-sep" />

        <button
          className="tile bleed"
          data-tip="Launchpad"
          data-sub="· toutes les sources"
          aria-label="Launchpad"
          onClick={onOpenLaunchpad}
        >
          <img src={ICONS.launch.src} alt="" />
        </button>

        <span className="dock-sep" />

        <button
          className="tile"
          data-tip={view === "map" ? "Vue mosaïque" : "Vue mind map"}
          data-sub="· changer de vue"
          aria-label="Changer de vue"
          onClick={onToggleView}
          style={{ color: "var(--color-muted)" }}
        >
          {view === "map" ? <LayoutGrid className="size-[22px]" /> : <Network className="size-[22px]" />}
        </button>

        {onCreate && (
          <button
            className="tile"
            data-tip="Nouvelle page"
            data-sub="· ou importer un fichier"
            aria-label="Nouvelle page"
            onClick={onCreate}
            style={{ color: "var(--color-accent)" }}
          >
            <Plus className="size-[22px]" />
          </button>
        )}
      </div>
    </div>
  );
}
