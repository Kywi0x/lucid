// Vérifie la lecture ciblée de `brain_node` (le poste à 93 % du coût en tokens).
// Aucune donnée réelle : contenus fabriqués sur place.
//
//   node scripts/check-mcp-passages.ts        (Node ≥ 22 retire les types tout seul)

import { relevantPassages, toolNode } from "../supabase/functions/lucid-mcp/index.ts";

const ok = (cond: unknown, msg: string) => {
  if (!cond) throw new Error(`ÉCHEC : ${msg}`);
  console.log(`ok — ${msg}`);
};

// 1. Markdown long : on ne veut que la section qui parle du sujet.
const md = [
  "# Contrat de bail",
  "\n## Loyer\nLe loyer est de 850 euros charges comprises.".padEnd(2000, " ."),
  "\n## Depot de garantie\nLe depot equivaut a un mois de loyer.".padEnd(2000, " ."),
  "\n## Assurance\nUne attestation est exigee chaque annee.".padEnd(2000, " ."),
].join("\n");
const p1 = relevantPassages(md, "depot de garantie")!;
ok(p1.includes("depot equivaut"), "markdown : le passage pertinent est renvoyé");
ok(!p1.includes("attestation est exigee"), "markdown : les sections hors sujet sont écartées");
ok(p1.length < md.length / 2, `markdown : ${p1.length} car. au lieu de ${md.length}`);

// 2. Export CSV : une seule ligne géante, pas de saut de ligne à couper.
const csv = "id,client,montant\n" +
  Array.from({ length: 400 }, (_, i) => `${i},client${i},${i * 7}`).join(";") +
  ";999,PLOMBERIE-URGENCE,1240";
const p2 = relevantPassages(csv, "plomberie")!;
ok(p2.includes("PLOMBERIE-URGENCE,1240"), "csv : la ligne cherchée est retrouvée dans une ligne unique");
ok(p2.length <= 3_200, `csv : budget respecté (${p2.length} car. pour ${csv.length})`);

// 3. Accents pliés dans les deux sens, et termes trop courts ignorés.
ok(relevantPassages("Le relevé géographique de 2019.".padEnd(4000, " ."), "geographique") !== null,
  "accents : « geographique » trouve « géographique »");
ok(relevantPassages("La GÉOGRAPHIE du site.".padEnd(4000, " ."), "géographie") !== null,
  "accents : requête accentuée trouve le contenu en majuscules");
ok(relevantPassages("x".repeat(4000), "de la") === null, "aucun terme utile (≤ 3 lettres) → null");
ok(relevantPassages("rien de commun ici".padEnd(4000, " ."), "zzz introuvable") === null,
  "aucun match → null (l'appelant renvoie le début de page)");

// 4. toolNode : les 3 régimes.
const big = "Debut de la page. ".padEnd(30_000, "bla ") + " MOT-CIBLE en fin de page.";
const payload = { title: "t", nodes: [{ id: "n1", label: "Page", kind: "leaf", content: big }] };
const targeted = toolNode(payload, "n1", { query: "mot-cible" });
const blind = toolNode(payload, "n1");
const full = toolNode(payload, "n1", { full: true });
ok(targeted.includes("MOT-CIBLE en fin de page"), "toolNode ciblé : atteint la fin d'une page de 30 k");
ok(targeted.length < 4_000, `toolNode ciblé : ${targeted.length} car.`);
ok(blind.length < 9_000 && blind.includes("Debut de la page"), `toolNode sans query : début de page (${blind.length} car.)`);
ok(full.length > 24_000, `toolNode full: true : page entière plafonnée (${full.length} car.)`);

// 5. Page courte : rien à économiser, elle passe intégralement.
const small = { title: "t", nodes: [{ id: "s", label: "Note", kind: "leaf", content: "Trois lignes.\nRien de plus." }] };
ok(toolNode(small, "s", { query: "absent" }).includes("Rien de plus."), "page courte : renvoyée telle quelle");

// 6. Page-conteneur : la question d'agrégation doit tenir en UN appel.
const facture = (i: number) =>
  `Facture ${i}\n`.padEnd(1600, "en-tete ") + `\nMontant TTC : ${100 + i},50 EUR\nFournisseur : OVH\n`.padEnd(1600, "pied ");
const dossier = {
  title: "t",
  nodes: [
    { id: "dir", label: "Factures Services", kind: "container", content: "Liste des factures." },
    ...Array.from({ length: 16 }, (_, i) => ({
      // `summary: ""` et pas d'omission : c'est la forme RÉELLE du payload (les
      // String Rust vides se sérialisent en chaîne vide). Une fixture qui omettait
      // le champ laissait passer un `??` au lieu d'un `||` — bug vu en prod le
      // 2026-08-06, sous-pages rendues nues.
      id: `f${i}`, label: `Invoice-${i}`, kind: "leaf", parent_id: "dir", summary: "",
      date: `2025-${String(i + 1).padStart(2, "0")}-01`, content: facture(i),
    })),
  ],
};
const agg = toolNode(dossier, "dir", { query: "montant fournisseur" });
ok(agg.includes("2025-01-01") && agg.includes("2025-16-01"), "conteneur : les dates des sous-pages sont exposées");
ok(agg.includes("Montant TTC : 100,50"), "conteneur : le montant d'un enfant remonte sans second appel");
ok((agg.match(/Montant TTC/g) ?? []).length >= 10, "conteneur : la majorité des enfants sont servis, pas un seul");
ok(agg.length < 12_000, `conteneur : budget tenu (${agg.length} car. pour 16 enfants de 3 200)`);

// Sans `query` — le cas réel : ChatGPT n'en passe pas (2026-08-06). Les extraits
// doivent descendre quand même, sinon le client repart avec la seule arborescence.
const blindAgg = toolNode(dossier, "dir");
ok(blindAgg.includes("Facture 0"), "conteneur sans query : le contenu des enfants descend quand même");
ok(blindAgg.includes("2025-01-01"), "conteneur sans query : les dates sont là");
// Le fournisseur est en tête, le total en pied : les deux doivent survivre à l'aperçu.
ok(blindAgg.includes("Montant TTC : 100,50"), "conteneur sans query : le total en pied de document remonte aussi");
ok(blindAgg.includes("Fournisseur : OVH"), "conteneur sans query : le pied complet est servi, pas seulement l'en-tête");

// Facture à l'américaine : symbole AVANT le nombre, aucune unité derrière.
const usd = {
  title: "t",
  nodes: [
    { id: "d2", label: "Factures Xano", kind: "container", content: "Liste." },
    { id: "u1", label: "Invoice-US", kind: "leaf", parent_id: "d2", summary: "",
      content: "Xano, Inc.\n".padEnd(1200, "header ") + "\nAmount due $25.00\n".padEnd(1200, "terms ") },
  ],
};
ok(toolNode(usd, "d2").includes("$25.00"), "conteneur sans query : montant en dollars, symbole devant le nombre");
ok(blindAgg.length < 12_000, `conteneur sans query : budget tenu (${blindAgg.length} car.)`);

console.log("\nTous les checks passent.");
