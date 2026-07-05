#!/bin/bash
# ─── Lucid — prépare les sidecars du bundle ─────────────────────────────────
# Collecte tous les binaires externes (llama-completion statique, lucid_mcp,
# pdftotext, pdftoppm, tesseract) + leurs dylibs + tessdata, aux emplacements
# attendus par tauri.conf.json (externalBin + resources).
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
LLAMA_STATIC="$HOME/Library/Application Support/fr.ideeri.brainlink/llama.cpp/build-static/bin/llama-completion"

rm -rf "$BIN_DIR" "$LIB_DIR" "$TESS_DIR"
mkdir -p "$BIN_DIR" "$LIB_DIR" "$TESS_DIR"

echo "── 1/5 lucid_mcp (release)"
(cd src-tauri && cargo build --release --bin lucid_mcp --quiet)
cp "src-tauri/target/release/lucid_mcp" "$BIN_DIR/lucid_mcp-$TRIPLE"

echo "── 2/5 llama-completion"
# Local : build statique (self-contained, pas de dylib). CI : release officielle
# llama.cpp (dynamique → ses dylibs seront embarqués par dylibbundler ci-dessous).
LLAMA_DYLIBS=""
if [ -f "$LLAMA_STATIC" ]; then
  echo "   → build statique local"
  cp "$LLAMA_STATIC" "$BIN_DIR/llama-completion-$TRIPLE"
else
  echo "   → download release officielle (macos-arm64)"
  url=$(curl -fsSL https://api.github.com/repos/ggml-org/llama.cpp/releases/latest \
    | grep -o 'https://[^"]*bin-macos-arm64\.zip' | head -1)
  [ -n "$url" ] || { echo "❌ asset llama.cpp macos-arm64 introuvable"; exit 1; }
  tmp=$(mktemp -d); curl -fsSL "$url" -o "$tmp/llama.zip"; unzip -q "$tmp/llama.zip" -d "$tmp/x"
  cli=$(find "$tmp/x" -name llama-cli -type f | head -1)
  [ -n "$cli" ] || { echo "❌ llama-cli introuvable dans l'archive"; exit 1; }
  cp "$cli" "$BIN_DIR/llama-completion-$TRIPLE"
  LLAMA_DYLIBS="llama-completion"   # → à passer dans dylibbundler
fi

echo "── 3/5 poppler (pdftotext + pdftoppm) + dylibs"
for b in pdftotext pdftoppm; do
  cp "/opt/homebrew/bin/$b" "$BIN_DIR/$b-$TRIPLE"
done

echo "── 4/5 tesseract + dylibs + tessdata (fra+eng+osd)"
cp /opt/homebrew/bin/tesseract "$BIN_DIR/tesseract-$TRIPLE"
for lang in fra eng osd; do
  cp "/opt/homebrew/share/tessdata/$lang.traineddata" "$TESS_DIR/"
done

# dylibbundler : copie la fermeture de dylibs dans resources/libs et réécrit
# les chemins en @executable_path/../Resources/libs/ (les sidecars finissent
# dans Contents/MacOS/, les libs dans Contents/Resources/libs/).
for b in pdftotext pdftoppm tesseract $LLAMA_DYLIBS; do
  echo "   dylibbundler: $b"
  dylibbundler -of -b -x "$BIN_DIR/$b-$TRIPLE" \
    -d "$LIB_DIR/" -p "@executable_path/../Resources/libs/" \
    -s /opt/homebrew/lib < /dev/null > /dev/null
done

echo "── 5/5 re-signature ad-hoc (dylibbundler invalide les signatures)"
codesign -f -s - "$BIN_DIR"/*-"$TRIPLE" "$LIB_DIR"/*.dylib 2>/dev/null

echo
echo "✅ Sidecars prêts :"
ls -lh "$BIN_DIR" | awk '{print "   " $9 " " $5}' | grep -v "^   $"
echo "   libs : $(ls "$LIB_DIR" | wc -l | tr -d ' ') dylibs ($(du -sh "$LIB_DIR" | cut -f1))"
echo "   tessdata : $(ls "$TESS_DIR" | tr '\n' ' ')"
