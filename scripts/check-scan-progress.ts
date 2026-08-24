// Vérifie la barre de progression du premier scan : l'extrapolation du temps
// restant et sa mise en mots. Aucune donnée réelle, aucun réseau.
//
//   node scripts/check-scan-progress.ts        (Node ≥ 22 retire les types tout seul)

import { etaSeconds, humanEta } from "../src/lib/utils.ts";

const ok = (cond: unknown, msg: string) => {
  if (!cond) throw new Error(`ÉCHEC : ${msg}`);
  console.log(`ok — ${msg}`);
};

const t0 = 1_000_000;

// 1. Trop tôt = on n'annonce RIEN. C'est le cœur de « honnête » : une
//    extrapolation sur deux points saute d'une minute à une heure.
ok(etaSeconds({ at: t0, current: 0 }, 2, 100, t0 + 30_000) === null,
  "moins de 3 éléments traités : aucun temps annoncé");
ok(etaSeconds({ at: t0, current: 0 }, 50, 100, t0 + 2_000) === null,
  "moins de 5 s de mesure : aucun temps annoncé");

// 2. Débit mesuré → reste. 10 éléments en 10 s, 90 restants = 90 s.
ok(etaSeconds({ at: t0, current: 0 }, 10, 100, t0 + 10_000) === 90,
  "10 éléments en 10 s sur 100 → 90 s restantes");
// Deux fois plus lent, deux fois plus long.
ok(etaSeconds({ at: t0, current: 0 }, 10, 100, t0 + 20_000) === 180,
  "débit divisé par deux → temps doublé");
// L'étape a démarré à un compteur non nul (reprise de cache) : on ne compte que
// ce qui a vraiment été observé, pas l'avance offerte au départ.
ok(etaSeconds({ at: t0, current: 400 }, 410, 500, t0 + 10_000) === 90,
  "départ à 400/500 : seuls les 10 éléments observés servent au calcul");

// 3. Fin atteinte ou dépassée : rien à annoncer (jamais de « reste 0 s »).
ok(etaSeconds({ at: t0, current: 0 }, 100, 100, t0 + 10_000) === null,
  "étape terminée : aucun temps annoncé");

// 4. Mise en mots : grossière exprès, jamais fausse-précise.
ok(humanEta(10) === "moins d'1 min", "10 s → « moins d'1 min »");
ok(humanEta(44) === "moins d'1 min", "44 s → « moins d'1 min » (seuil)");
ok(humanEta(120) === "~2 min", "120 s → « ~2 min »");
ok(humanEta(3900) === "~1 h 5 min", "3 900 s → « ~1 h 5 min »");
ok(!humanEta(107).includes("47"), "pas de fausse précision à la seconde");

console.log("\n✅ barre de progression : 11 vérifications passées");
