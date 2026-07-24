// Mesure combien de caractères (≈ tokens) le MCP distant renvoie réellement
// vs la taille du cerveau complet — jamais de contenu affiché, que des tailles.
//
// Usage :
//   deno run --allow-read scripts/measure-mcp-token-savings.ts <payload.json> "<recherche>" <node_id...>
//
// <payload.json> : un export réel d'un space (même forme que ce que lit lucid-mcp :
// { title, nodes: [{ id, label, kind, summary?, keywords?, parent_id?, content? }, ...] }).
// Ne commite jamais ce fichier — reste en local, n'est lu que par ce script.

import { toolOverview, toolSearch, toolNode } from "../supabase/functions/lucid-mcp/index.ts";

const [payloadPath, query, ...nodeIds] = Deno.args;
if (!payloadPath) {
  console.error("Usage: deno run --allow-read measure-mcp-token-savings.ts <payload.json> [query] [node_id...]");
  Deno.exit(1);
}

const raw = await Deno.readTextFile(payloadPath);
const payload = JSON.parse(raw);

const approxTokens = (chars: number) => Math.round(chars / 4);

const fullChars = raw.length;
console.log(`Payload complet   : ${fullChars.toLocaleString()} caractères (~${approxTokens(fullChars).toLocaleString()} tokens)`);
console.log(`Nb de pages       : ${payload.nodes.length}`);

const overview = toolOverview(payload);
console.log(`\nbrain_overview    : ${overview.length.toLocaleString()} caractères (~${approxTokens(overview.length).toLocaleString()} tokens)`);

let searchChars = 0;
if (query) {
  const search = toolSearch(payload, query);
  searchChars = search.length;
  console.log(`brain_search("${query}") : ${search.length.toLocaleString()} caractères (~${approxTokens(search.length).toLocaleString()} tokens)`);
}

let nodeChars = 0;
for (const id of nodeIds) {
  try {
    const node = toolNode(payload, id);
    nodeChars += node.length;
    console.log(`brain_node(${id})  : ${node.length.toLocaleString()} caractères (~${approxTokens(node.length).toLocaleString()} tokens)`);
  } catch (e) {
    console.log(`brain_node(${id})  : erreur — ${(e as Error).message}`);
  }
}

const scenarioChars = overview.length + searchChars + nodeChars;
console.log(`\nScénario réaliste (1 overview + 1 search + ${nodeIds.length} node) : ${scenarioChars.toLocaleString()} caractères (~${approxTokens(scenarioChars).toLocaleString()} tokens)`);
console.log(`vs payload complet : ${fullChars.toLocaleString()} caractères`);
console.log(`Ratio : ${(fullChars / Math.max(scenarioChars, 1)).toFixed(1)}x moins de caractères envoyés via MCP.`);
