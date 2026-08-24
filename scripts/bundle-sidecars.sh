#!/bin/bash
# ─── Lucid — prépare les sidecars du bundle ─────────────────────────────────
# Collecte tous les binaires externes (llama-completion statique, pdftotext,
# pdftoppm, tesseract) + leurs dylibs + tessdata, aux emplacements attendus
# par tauri.conf.json (externalBin + resources). Le MCP est désormais 100%
# distant (supabase/functions/lucid-mcp) — plus de binaire local à bundler.
#
# Prérequis : brew install poppler tesseract tesseract-lang dylibbundler
#             + llama.cpp compilé en statique (build-static) — voir journal.
# Usage : ./scripts/bundle-sidecars.sh
set -euo pipefail
cd "$(dirname "$0")/.."

TRIPLE=$(rustc -vV | awk '/^host:/ {print $2}')
BIN_DIR="src-tauri/binaries"
LIB_DIR="src-tauri/resources/libs"
TESS_DIR="src-tauri/resources/tessdata"
LLAMA_STATIC="$HOME/Library/Application Support/com.lucidflow.lucid/llama.cpp/build-static/bin/llama-completion"
# llama-server vit à côté de llama-completion (même build) : sert le serveur
# persistant ET les embeddings (cf. resolve_server_binary). Bundlé en sidecar.
LLAMA_SERVER_STATIC="$(dirname "$LLAMA_STATIC")/llama-server"

rm -rf "$BIN_DIR" "$LIB_DIR" "$TESS_DIR"
mkdir -p "$BIN_DIR" "$LIB_DIR" "$TESS_DIR"

# tauri-build (build.rs) valide l'existence de TOUS les externalBin dès qu'on
# compile le crate. On crée des placeholders vides pour satisfaire cette
# vérification ; les vrais binaires les écrasent aux étapes suivantes.
for b in llama-completion llama-server pdftotext pdftoppm tesseract; do
  : > "$BIN_DIR/$b-$TRIPLE"
done

echo "── 1/4 llama-completion"
# Local : build statique (self-contained, pas de dylib) — SEULEMENT si les DEUX
# binaires (completion + server) y sont. Sinon (ex. build statique partiel, ou
# CMakeCache périmé après le renommage du dossier app), on bascule sur la release
# officielle llama.cpp comme le fait la CI (dynamique → dylibs embarqués par
# dylibbundler ci-dessous) : versions cohérentes entre les deux binaires.
LLAMA_DYLIBS=""; LLAMA_SRC=""
if [ -f "$LLAMA_STATIC" ] && [ -f "$LLAMA_SERVER_STATIC" ]; then
  echo "   → build statique local (completion + server)"
  cp "$LLAMA_STATIC" "$BIN_DIR/llama-completion-$TRIPLE"
  cp "$LLAMA_SERVER_STATIC" "$BIN_DIR/llama-server-$TRIPLE"
else
  echo "   → download release officielle (macos-arm64, .tar.gz)"
  # Auth GitHub API si dispo (évite le rate-limit anonyme sur les runners partagés).
  # ${a[@]+"${a[@]}"} = expansion sûre d'un array vide sous `set -u` (bash 3.2 macOS).
  auth=(); [ -n "${GITHUB_TOKEN:-}" ] && auth=(-H "Authorization: Bearer $GITHUB_TOKEN")
  api=https://api.github.com/repos/ggml-org/llama.cpp/releases/latest
  url=$(curl -fsSL ${auth[@]+"${auth[@]}"} "$api" \
    | grep -o 'https://[^"]*bin-macos-arm64\.tar\.gz' | head -1 || true)
  [ -n "$url" ] || { echo "❌ asset llama.cpp macos-arm64 introuvable"; exit 1; }
  tmp=$(mktemp -d); curl -fsSL ${auth[@]+"${auth[@]}"} "$url" -o "$tmp/llama.tgz"
  tar -xzf "$tmp/llama.tgz" -C "$tmp"
  cli=$(find "$tmp" -name llama-cli -type f | head -1)
  [ -n "$cli" ] || { echo "❌ llama-cli introuvable dans l'archive"; exit 1; }
  cp "$cli" "$BIN_DIR/llama-completion-$TRIPLE"
  # llama-server est dans la même archive (mêmes dylibs @rpath).
  srv=$(find "$tmp" -name llama-server -type f | head -1)
  [ -n "$srv" ] || { echo "❌ llama-server introuvable dans l'archive"; exit 1; }
  cp "$srv" "$BIN_DIR/llama-server-$TRIPLE"
  LLAMA_DYLIBS="llama-completion"      # → à passer dans dylibbundler
  LLAMA_SRC=$(dirname "$cli")          # dossier des dylibs @rpath (libllama, libggml…)
fi

echo "── 2/4 poppler (pdftotext + pdftoppm) + dylibs"
for b in pdftotext pdftoppm; do
  cp "/opt/homebrew/bin/$b" "$BIN_DIR/$b-$TRIPLE"
done

echo "── 3/4 tesseract + dylibs + tessdata (fra+eng+osd)"
cp /opt/homebrew/bin/tesseract "$BIN_DIR/tesseract-$TRIPLE"
for lang in fra eng osd; do
  cp "/opt/homebrew/share/tessdata/$lang.traineddata" "$TESS_DIR/"
done

# dylibbundler : copie la fermeture de dylibs dans resources/libs et réécrit
# les chemins en @executable_path/../Resources/libs/ (les sidecars finissent
# dans Contents/MacOS/, les libs dans Contents/Resources/libs/).
for b in pdftotext pdftoppm tesseract; do
  echo "   dylibbundler: $b"
  dylibbundler -of -b -x "$BIN_DIR/$b-$TRIPLE" \
    -d "$LIB_DIR/" -p "@executable_path/../Resources/libs/" \
    -s /opt/homebrew/lib < /dev/null > /dev/null
done
# llama téléchargé (dynamique) : ses dylibs @rpath sont dans l'archive → -s LLAMA_SRC.
# (build statique local : LLAMA_DYLIBS vide, rien à faire.)
if [ -n "$LLAMA_DYLIBS" ]; then
  for lb in llama-completion llama-server; do
    echo "   dylibbundler: $lb"
    dylibbundler -of -b -x "$BIN_DIR/$lb-$TRIPLE" \
      -d "$LIB_DIR/" -p "@executable_path/../Resources/libs/" \
      -s /opt/homebrew/lib -s "$LLAMA_SRC" < /dev/null > /dev/null
  done
fi

echo "── 4/4 bit exécutable + re-signature ad-hoc (dylibbundler invalide les signatures)"
chmod +x "$BIN_DIR"/*-"$TRIPLE"   # le llama téléchargé perd parfois son +x après cp/bundle
# Signature ad-hoc des sidecars et de leurs dylibs. Elle ne SUFFIT PAS : Tauri
# re-signe ensuite les binaires de Contents/MacOS/ avec le hardened runtime, ce
# qui active la « library validation » — le process refuse alors toute dylib
# signée séparément, et les nôtres vivent dans Contents/Resources/libs/.
#
# Symptôme (mesuré le 2026-08-21 sur une app installée, machine de dev ET machine
# tierce) : les CINQ sidecars morts au lancement — llama-completion, llama-server,
# pdftotext, pdftoppm, tesseract. Donc zéro IA locale, zéro OCR, zéro vecteur, sur
# toutes les versions distribuées jusque-là (beta.17 comprise). Invisible en dev :
# hors bundle, rien n'est signé avec le hardened runtime.
#
# Le message d'erreur ment : il parle de « different Team IDs » alors que les deux
# côtés sont en ad-hoc, Team ID vide. La vraie cause est la library validation.
#
# Correctif : src-tauri/entitlements.plist (disable-library-validation), branché
# dans tauri.macos.conf.json. Vérifier après un build :
#   codesign -d --verbose=4 <app>/Contents/MacOS/llama-server   → doit lister l'entitlement
#   <app>/Contents/MacOS/llama-server --version                 → doit répondre
#
# Pas de 2>/dev/null ici : une signature qui échoue doit se voir (ADR-0015).
codesign -f -s - "$BIN_DIR"/*-"$TRIPLE" "$LIB_DIR"/*.dylib

echo
echo "✅ Sidecars prêts :"
ls -lh "$BIN_DIR" | awk '{print "   " $9 " " $5}' | grep -v "^   $"
echo "   libs : $(ls "$LIB_DIR" | wc -l | tr -d ' ') dylibs ($(du -sh "$LIB_DIR" | cut -f1))"
echo "   tessdata : $(ls "$TESS_DIR" | tr '\n' ' ')"
