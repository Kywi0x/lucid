/** Contrôle du pavage mosaïque : les invariants que le layout doit tenir.
 *
 *  Usage : node scripts/check-mosaic.mjs [chemin/vers/brain.json]
 *  Sans argument, prend le brain.json du dossier de données de l'app.
 *
 *  ponytail: pas de framework de test — un script, des `assert`, et il tourne
 *  sur le vrai graphe. C'est ce qui casse si le pavage régresse.
 */
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync, mkdtempSync } from "node:fs";
import { tmpdir, homedir } from "node:os";
import { join } from "node:path";

const TILE = 10, G_NOTE = 2, G_SUB = 6, G_BIG = 24, ROOT_R = 56;

// on compile le module TS à la volée (esbuild vient déjà avec vite)
const out = join(mkdtempSync(join(tmpdir(), "mosaic-")), "mosaic.mjs");
execFileSync("npx", ["esbuild", "src/lib/mosaicLayout.ts", "--bundle", "--format=esm", "--outfile=" + out], {
  stdio: ["ignore", "ignore", "inherit"],
});
const { buildMosaic } = await import(out);

const path = process.argv[2] ??
  join(homedir(), "Library/Application Support/com.lucidflow.lucid/brain.json");
const graph = JSON.parse(readFileSync(path, "utf8"));
assert.ok(Array.isArray(graph.nodes) && graph.nodes.length, "graphe vide");

const L = buildMosaic(graph);
const kids = new Map();
for (const n of graph.nodes) {
  if (n.kind === "pending" || !n.parent_id) continue;
  kids.set(n.parent_id, (kids.get(n.parent_id) ?? 0) + 1);
}
const leaves = graph.nodes.filter((n) => n.kind !== "pending" && n.parent_id && !kids.has(n.id));

console.log(
  `${graph.nodes.length} nœuds → ${L.tiles.length} tuiles, ${L.folders.length} dossiers, ` +
  `${L.absorbed} absorbés, ${L.missing} manquantes, ${L.detached} détachés`
);

// 1. aucune note perdue en silence
assert.equal(L.missing, 0, `${L.missing} notes non posées`);
assert.equal(L.detached, 0, `${L.detached} dossiers sans place`);

// 2. une tuile par feuille, une par dossier
const noteTiles = L.tiles.filter((t) => !t.isFolder);
assert.equal(noteTiles.length, leaves.length, "feuilles ≠ tuiles-notes");
assert.equal(L.tiles.length - noteTiles.length, L.folders.length - 1, "dossiers ≠ tuiles-dossiers");

// 3. aucun recouvrement, et la grammaire des écarts est respectée partout
const spaceOf = (f) => { let c = f; while (c.parent && c.parent.parent) c = c.parent; return c; };
let checked = 0;
for (let i = 0; i < L.tiles.length; i++) {
  const a = L.tiles[i];
  // le centre est réservé : personne n'entre dans le disque du root
  assert.ok(Math.hypot(a.x + TILE / 2, a.y + TILE / 2) > ROOT_R, "tuile dans le disque du root");
  for (let j = i + 1; j < L.tiles.length; j++) {
    const b = L.tiles[j];
    const sep = Math.max(Math.abs(a.x - b.x) - TILE, Math.abs(a.y - b.y) - TILE);
    if (sep >= G_BIG) continue;                       // hors de portée, rien à dire
    const need = a.cid !== b.cid ? G_BIG
               : a.folder !== b.folder ? G_SUB
               : G_NOTE;
    assert.ok(sep >= need, `écart ${sep} < ${need} attendu (cid ${a.cid}/${b.cid})`);
    checked++;
  }
}

// 4. l'aplatissement est allé jusqu'au point fixe
for (const f of L.folders) {
  if (!f.parent) continue;
  assert.ok(!(f.notes.length === 0 && f.children.length === 1),
    `dossier de passage restant : ${f.label}`);
}

console.log(`OK — ${checked} paires proches vérifiées, grammaire 2/6/24 respectée`);
