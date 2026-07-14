import type { BrainGraph, BrainNode } from "./types";

/** Payload `shared_spaces.data` → BrainGraph prêt pour BrainMap.
 *  Pur (aucun client) : utilisé par l'app ET le viewer web. */
export function payloadToGraph(data: { nodes: Partial<BrainNode>[]; edges?: BrainGraph["edges"] }): BrainGraph {
  const nodes: BrainNode[] = data.nodes.map((n) => ({
    summary: "", keywords: [], decisions: [], patterns: [], content: "",
    ...n,
  } as BrainNode));
  return { nodes, edges: data.edges ?? [], markdown: "", report: "", generated_at: "" };
}
