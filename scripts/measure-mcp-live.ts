// Mesure sur le MCP DÉPLOYÉ ce que coûte un parcours de question réel, et compare
// `brain_node` avec `query` (passages ciblés) vs sans (début de page).
// N'affiche que des TAILLES — aucun contenu, aucune donnée personnelle.
//
//   node scripts/measure-mcp-live.ts "<url-mcp-avec-token>" "<question>"
//
// L'URL contient un token : passe-la en argument, ne la commite pas.

const [url, query] = process.argv.slice(2);
if (!url || !query) {
  console.error('Usage: node scripts/measure-mcp-live.ts "<url-mcp>" "<question>"');
  process.exit(1);
}

const call = async (name: string, args: Record<string, unknown>): Promise<string> => {
  const r = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name, arguments: args } }),
  });
  const j = await r.json();
  const text = j.result?.content?.[0]?.text;
  if (typeof text !== "string") throw new Error(`réponse inattendue : ${JSON.stringify(j).slice(0, 200)}`);
  return text;
};

const tok = (chars: number) => Math.round(chars / 4);
const overview = await call("brain_overview", {});
const search = await call("brain_search", { query });
const ids = [...search.matchAll(/\(`([^`]+)`, /g)].map((m) => m[1]).slice(0, 2);

console.log(`brain_overview        ~${tok(overview.length)} tok`);
console.log(`brain_search          ~${tok(search.length)} tok`);
if (!ids.length) console.log("(aucun résultat — rien à lire)");

let blind = 0, targeted = 0;
for (const id of ids) {
  const a = (await call("brain_node", { node_id: id })).length;
  const b = (await call("brain_node", { node_id: id, query })).length;
  blind += a;
  targeted += b;
  console.log(`brain_node            ~${tok(a)} tok sans query → ~${tok(b)} tok avec query   (${id.slice(0, 48)})`);
}

const base = tok(overview.length + search.length);
console.log(`\nTotal du parcours     ~${base + tok(blind)} tok sans query → ~${base + tok(targeted)} tok avec query`);
// Depuis que les conteneurs descendent leurs sous-pages, `query` peut RENCHÉRIR la
// lecture d'une page : +1 200 tok sur un appel qui en remplace 16. Le signe compte,
// pas la valeur absolue — d'où les deux formulations.
if (blind) {
  const delta = Math.round((1 - targeted / blind) * 100);
  console.log(delta >= 0
    ? `Lecture de page       ${delta} % en moins`
    : `Lecture de page       ${-delta} % en plus — un conteneur sert ses sous-pages, et économise autant d'appels`);
}

// Une page-conteneur doit servir ses sous-pages (date + extrait) en UN appel :
// c'est ce qui décide si une question d'agrégation coûte 1 aller-retour ou N.
// On ne compte que la FORME des lignes rendues — rien n'est affiché du contenu.
console.log("\nSous-pages servies par les conteneurs :");
let container = 0, anyDated = 0;
for (const id of ids) {
  const page = await call("brain_node", { node_id: id, query });
  const section = page.split("\n## Sous-pages\n")[1];
  if (!section) continue;
  container++;
  const lines = section.split("\n");
  const kids = lines.filter((l) => l.startsWith("- **")).length;
  const dated = lines.filter((l) => l.startsWith("- **") && / · \d{4}-/.test(l)).length;
  const peeks = lines.filter((l) => l.startsWith("  ") && l.trim()).length;
  anyDated += dated;
  console.log(`  ${id.slice(0, 40)} : ${kids} sous-pages · ${dated} datées · ${peeks} avec extrait`);
  // 0 extrait sur un conteneur hors sujet est NORMAL : aucun enfant ne matche la
  // requête. Le vrai signal de « ancienne version déployée », c'est l'absence de
  // date PARTOUT — date et extraits sont arrivés dans le même déploiement.
  if (!peeks) console.log("    (aucun enfant ne matche la requête — attendu si ce dossier est hors sujet)");
}
if (container && !anyDated) console.log("\n⚠ aucune date nulle part : soit la fonction déployée est l'ancienne, soit le payload stocké est antérieur au champ `date` — republie le space depuis Lucid.");
if (!container) console.log("  (aucun conteneur dans les résultats — vise une question qui tombe sur un dossier)");
