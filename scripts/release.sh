#!/usr/bin/env bash
# Publier une version de Lucid — sans réfléchir aux numéros.
#
#   npm run release -- beta     → nouvelle beta (testeurs)
#   npm run release -- stable   → promeut la beta courante (tout le monde)
#
# Le script trouve la dernière version, calcule la suivante, tague et pousse.
# Le CI fait le reste (build Mac + Windows, canaux de mise à jour, site).
set -euo pipefail

mode="${1:-}"
if [[ "$mode" != "beta" && "$mode" != "stable" ]]; then
  echo "Usage : npm run release -- beta|stable"
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "⚠️  Modifs non commitées — commit + push d'abord, puis relance."
  exit 1
fi

git fetch -q --tags
# Dernière version publiée = le tag le plus récemment créé (pas de tri semver piégeux).
last=$(git tag -l 'v*' --sort=-creatordate | head -1)
base="${last#v}"

if [[ "$mode" == "beta" ]]; then
  if [[ "$base" == *-beta.* ]]; then
    n="${base##*-beta.}"
    next="v${base%-beta.*}-beta.$((n + 1))"
  else
    IFS=. read -r maj min _ <<<"$base"
    next="v${maj}.$((min + 1)).0-beta.1"
  fi
else
  if [[ "$base" != *-beta.* ]]; then
    echo "⚠️  Rien à promouvoir : la dernière version ($last) est déjà stable."
    echo "    Fais d'abord une beta (npm run release -- beta) et teste-la."
    exit 1
  fi
  next="v${base%-beta.*}"
fi

echo "Dernière version publiée : $last"
read -rp "Publier $next ($mode) ? [y/N] " ok
[[ "$ok" == "y" || "$ok" == "Y" ]] || { echo "Annulé."; exit 0; }

git push
git tag "$next"
git push origin "$next"
echo "✅ $next est parti. Build ~20 min : https://github.com/Kywi0x/lucid/actions"
echo "   Ensuite, les apps installées afficheront la bannière de mise à jour au prochain lancement."
