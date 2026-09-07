/// Contrôle du compteur d'économie (relevé d'activité, ADR-0025).
/// Le chiffre affiché à l'utilisateur est une soustraction : si elle est fausse,
/// on lui ment. Ce fichier est le garde-fou minimal.
///
///   node supabase/functions/lucid-mcp/check-usage.ts
///
/// Pas de framework : node ≥ 22 exécute le TypeScript directement.

import assert from "node:assert/strict";
import { toolNode, toolSearch, meterSnapshot } from "./index.ts";

const BLIND_BUDGET = 8_000;
const FULL_MAX = 24_000;
const TARGETED_BUDGET = 3_000;
const SEARCH_EXCERPT = 500;
// `truncate()` colle un marqueur visible en fin de coupe : il est réellement
// envoyé au modèle, donc réellement compté.
const TRUNC_MARK = "\n\n*[… tronqué]*".length;
// `relevantPassages` rend des BLOCS entiers : un budget plus petit qu'un bloc
// sort quand même un bloc. SEARCH_EXCERPT=500 plafonne donc à BLOCK_SIZE=800
// dans les faits — comportement existant, le compteur mesure le réel.
const BLOCK_SIZE = 800;

/// Mesure ce qu'UN appel a compté (delta de snapshots : pas besoin d'exposer
/// le reset, qui reste interne au module).
function measure(fn: () => void): { raw: number; served: number } {
  const before = meterSnapshot();
  fn();
  const after = meterSnapshot();
  return { raw: after.raw - before.raw, served: after.served - before.served };
}

const long = "Le loyer de mars est de 1 250 € pour le local. ".repeat(2_000); // ~92 000 c.
const short = "Une note courte.";

// deno-lint-ignore no-explicit-any
const space = (content: string): any => ({
  title: "Contrôle",
  nodes: [{ id: "n1", label: "Bail", kind: "note", summary: "Bail commercial", keywords: ["loyer"], content }],
});

// 1. Page longue, sans query : on sert le début de page, on compte la page entière.
{
  const m = measure(() => toolNode(space(long), "n1"));
  assert.equal(m.raw, long.length, "raw = taille réelle de la page");
  assert.equal(m.served, BLIND_BUDGET + TRUNC_MARK, "served = budget aveugle + marqueur");
  assert.ok(m.served < m.raw, "il doit y avoir une économie");
}

// 2. `full: true` : plafonné, donc encore une économie sur une page énorme.
{
  const m = measure(() => toolNode(space(long), "n1", { full: true }));
  assert.equal(m.raw, long.length);
  assert.equal(m.served, FULL_MAX + TRUNC_MARK, "served = plafond historique + marqueur");
}

// 3. Page courte : servie telle quelle, aucune économie inventée.
{
  const m = measure(() => toolNode(space(short), "n1"));
  assert.equal(m.raw, m.served, "une page courte n'économise rien");
}

// 4. Avec query : passages ciblés, sous le budget.
{
  const m = measure(() => toolNode(space(long), "n1", { query: "loyer" }));
  assert.equal(m.raw, long.length);
  assert.ok(m.served > 0 && m.served <= TARGETED_BUDGET + TRUNC_MARK, `served=${m.served} hors budget ciblé`);
}

// 5. Recherche : un extrait par résultat, la page entière en face.
{
  const m = measure(() => toolSearch(space(long), "loyer"));
  assert.equal(m.raw, long.length, "raw = la page candidate entière");
  assert.ok(
    m.served <= Math.max(SEARCH_EXCERPT, BLOCK_SIZE) + TRUNC_MARK,
    `extrait=${m.served} trop long`,
  );
}

// 6. Le compteur s'ACCUMULE sur plusieurs pages d'un même appel.
{
  const p = space(long);
  p.nodes.push({ ...p.nodes[0], id: "n2", label: "Bail 2" });
  const m = measure(() => toolNode(p, "n1"));
  const m2 = measure(() => toolNode(p, "n2"));
  assert.equal(m.raw + m2.raw, long.length * 2, "deux lectures = deux pages comptées");
}

// 7. Le % affiché reste dans [0, 100].
{
  const m = measure(() => toolNode(space(long), "n1"));
  const pct = (1 - m.served / m.raw) * 100;
  assert.ok(pct > 0 && pct < 100, `pct=${pct} hors bornes`);
}

console.log("check-usage : 7 contrôles OK");
