// Sonde de vocabulaire : dit seulement si un mot est PRÉSENT ou ABSENT du cerveau,
// et combien de pages il touche. Aucun titre, aucun contenu, aucune donnée affichée.
//
//   node scripts/check-mcp-vocab.ts "<url-mcp>" mot1 mot2 ...
//
// À lancer AVANT chaque campagne de test (cf. docs/test-pertinence-mcp.md) : les mots
// de contrôle s'usent, le cerveau réindexe la documentation du test elle-même. Les
// quatre mots « vérifiés libres » du 2026-08-05 étaient tous grillés au 2026-08-06.
// Un mot touchant 1 seule page est un bon candidat « aiguille » ; 0 page, un bon
// contrôle négatif.
//
// L'URL contient un token : passe-la en argument, ne la commite pas.
const [url, ...words] = process.argv.slice(2);
if (!url || !words.length) { console.error("usage: node probe.ts <url> mot..."); process.exit(1); }

for (const w of words) {
  const r = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "brain_search", arguments: { query: w } } }),
  });
  const text = (await r.json()).result?.content?.[0]?.text ?? "";
  const absent = text.startsWith("Aucune des");
  const hits = (text.match(/^- \*\*/gm) ?? []).length;
  console.log(`${w.padEnd(24)} ${absent ? "ABSENT  ✅ utilisable comme contrôle" : `présent (${hits} page(s) dans le top 8)`}`);
}
