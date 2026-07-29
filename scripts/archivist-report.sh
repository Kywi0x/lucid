#!/usr/bin/env bash
# Rapport de diagnostic Archiviste — SANS contenu de documents (RGPD-safe).
# Produit un résumé texte à copier/coller pour analyse.
#
# Usage :
#   bash archivist-report.sh              # labels visibles (données non sensibles / démo)
#   MASK=1 bash archivist-report.sh       # labels masqués (T1, T2… + domaine + compteurs seuls)
#
# Ne lit QUE la structure (nodes: id/label/parent/kind) + les tags de domaine.
# N'imprime JAMAIS le contenu (source_text/content) ni d'extrait.

set -euo pipefail
APP="$HOME/Library/Application Support/com.lucidflow.lucid"
DB=$(find "$APP" -name brain.db -exec stat -f '%m %N' {} \; 2>/dev/null | sort -rn | head -1 | cut -d' ' -f2-)
[ -z "${DB:-}" ] && { echo "brain.db introuvable (l'app a-t-elle généré un cerveau ?)"; exit 1; }
DOM="$(dirname "$DB")/archivist_domains.json"
MASK="${MASK:-0}"

echo "==================== RAPPORT ARCHIVISTE ===================="
echo

echo "— Bilan de tri —"
sqlite3 "$DB" "WITH l AS (SELECT id,parent_id FROM nodes WHERE kind IN ('leaf','note'))
SELECT
  'documents totaux : '||(SELECT count(*) FROM l)||char(10)||
  'rangés en thèmes : '||(SELECT count(*) FROM l WHERE parent_id LIKE 'arch-theme-%' OR parent_id LIKE 'arch-cat-%')||char(10)||
  'Non triable      : '||(SELECT count(*) FROM l WHERE parent_id LIKE 'arch-non-triable%')||char(10)||
  'thèmes / catégories : '||(SELECT count(*) FROM nodes WHERE id LIKE 'arch-theme-%')||' / '||(SELECT count(*) FROM nodes WHERE id LIKE 'arch-cat-%');"
echo

echo "— Distribution des domaines (tags Gemma) —"
if [ -f "$DOM" ]; then
  node -e '
    const fs=require("fs");const d=JSON.parse(fs.readFileSync(process.argv[1],"utf8"));
    const c={};for(const k in d){const x=d[k].domain||"?";c[x]=(c[x]||0)+1;}
    for(const[k,v]of Object.entries(c).sort((a,b)=>b[1]-a[1]))console.log("  "+String(v).padStart(4)+"  "+k);
    console.log("  total taggé : "+Object.keys(d).length);
  ' "$DOM"
else echo "  (pas de cache de domaines)"; fi
echo

echo "— Structure (dossier → parent → nb docs → domaine majoritaire) —"
# domaine majoritaire par dossier, calculé côté SQL via jointure sur les enfants
sqlite3 -separator '|' "$DB" "
SELECT n.id, n.label, COALESCE(p.label,'RACINE'),
  (SELECT count(*) FROM nodes c WHERE c.parent_id=n.id)
FROM nodes n LEFT JOIN nodes p ON p.id=n.parent_id
WHERE n.id LIKE 'arch-%' ORDER BY COALESCE(p.label,'RACINE'), 4 DESC;" | \
node -e '
  const fs=require("fs");
  const dom = fs.existsSync(process.argv[1]) ? JSON.parse(fs.readFileSync(process.argv[1],"utf8")) : {};
  const mask = process.argv[2]==="1";
  const rows=fs.readFileSync(0,"utf8").trim().split("\n").filter(Boolean);
  let i=0;
  for(const line of rows){
    const [id,label,parent,k]=line.split("|");
    // domaine majoritaire = domaine le plus fréquent parmi les enfants (via cache)
    const name = mask ? ("Dossier "+(++i)) : label;
    const par  = mask ? (parent==="RACINE"?"RACINE":"[parent]") : parent;
    console.log("  "+String(k).padStart(4)+"  "+name+"  ← "+par);
  }
' "$DOM" "$MASK"
echo
echo "============================================================"
echo "Copie TOUT ce bloc et envoie-le. Aucun contenu de document n'y figure."
[ "$MASK" = "1" ] && echo "(mode masqué : noms de dossiers anonymisés)"
