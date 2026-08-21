# ─── Lucid — sidecars du bundle Windows ──────────────────────────────────────
# Prépare les binaires externes attendus par tauri.windows.conf.json :
#   - llama-completion.exe (= llama-cli.exe de la release officielle llama.cpp, build
#     Vulkan : GPU si le pilote le permet, repli CPU automatique sinon — cf. §1)
#     + les DLL de llama.cpp dans resources/win-libs/ (posées à côté de l'exe)
#   - poppler (pdftotext/pdftoppm + DLL) dans resources/win-poppler/ → bundle poppler/
#   - tesseract (+ DLL) dans resources/win-tesseract/               → bundle tesseract/
#   - tessdata fra+eng+osd dans resources/win-tessdata/             → bundle tessdata/
# Parité Mac/Windows (ADR-0015) : PDF texte, tableaux ET scannés (OCR) sur Windows.
# Le MCP est désormais 100% distant (supabase/functions/lucid-mcp) — plus de
# binaire local à compiler/bundler ici.
#
# Usage (runner windows-latest ou PC Windows) : pwsh scripts/bundle-sidecars.ps1
$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true   # cargo & co. : exit≠0 => stop
Set-Location (Join-Path $PSScriptRoot "..")

$Triple = "x86_64-pc-windows-msvc"
$BinDir = "src-tauri/binaries"
$LibDir = "src-tauri/resources/win-libs"
$PopDir = "src-tauri/resources/win-poppler"
$TesDir = "src-tauri/resources/win-tesseract"
$TdaDir = "src-tauri/resources/win-tessdata"
Remove-Item -Recurse -Force $BinDir, $LibDir, $PopDir, $TesDir, $TdaDir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $BinDir, $LibDir, $PopDir, $TesDir, $TdaDir | Out-Null

# Auth GitHub API si dispo (évite le rate-limit anonyme sur les runners partagés).
$ghHeaders = @{ "User-Agent" = "lucid-ci" }
if ($env:GITHUB_TOKEN) { $ghHeaders["Authorization"] = "Bearer $($env:GITHUB_TOKEN)" }

Write-Host "── 1/3 llama-completion (release officielle llama.cpp, Vulkan x64)"
# ARCHIVE VULKAN, pas CPU (A3, 2026-08-21). Le run Windows du 06/08 tournait à
# 8,76 tok/s en génération — du CPU pur — soit ~85 min pour un premier scan de
# 679 documents. C'est la différence entre « je lance et je reviens » et
# « j'abandonne ».
#
# Vulkan plutôt que CUDA : CUDA coûterait 251 Mo + 391 Mo de cudart et ne
# marcherait que sur NVIDIA. Vulkan fait 35 Mo (contre 18,6 en CPU, donc +16 Mo
# dans l'installeur) et couvre NVIDIA, AMD et Intel avec le pilote du système.
#
# Aucun risque de perdre le CPU : l'archive Vulkan contient AUSSI les 15 DLL
# `ggml-cpu-*` (vérifié le 21/08 sur b10549). ggml charge ses backends
# dynamiquement — sans GPU exploitable, `ggml-vulkan.dll` est simplement ignorée
# et la génération retombe sur le CPU. C'est un sur-ensemble, pas un échange.
#
# `latest` publie ses assets par vagues : on balaie les dernières releases
# jusqu'à en trouver une complète. Repli explicite sur l'archive CPU si aucune
# Vulkan n'est disponible — mieux vaut un build lent qu'un build cassé, et on
# le DIT (ADR-0015 : jamais d'échec silencieux).
$releases = Invoke-RestMethod "https://api.github.com/repos/ggml-org/llama.cpp/releases?per_page=10" -Headers $ghHeaders
$asset = $null
foreach ($r in $releases) {
    $asset = $r.assets | Where-Object { $_.name -like "*bin-win-vulkan-x64.zip" } | Select-Object -First 1
    if ($asset) { break }
}
if (-not $asset) {
    Write-Warning "Aucune archive Vulkan dans les 10 dernières releases → repli sur l'archive CPU."
    Write-Warning "Conséquence : génération ~6x plus lente sur GPU récent (8,76 tok/s mesurés en CPU pur)."
    foreach ($r in $releases) {
        $asset = $r.assets | Where-Object { $_.name -like "*bin-win-cpu-x64.zip" } | Select-Object -First 1
        if ($asset) { break }
    }
}
if (-not $asset) { throw "Aucun asset llama.cpp win-x64 (ni vulkan ni cpu) dans les 10 dernières releases." }
Write-Host "   → $($asset.name)"

$zip = Join-Path $env:TEMP $asset.name
$out = Join-Path $env:TEMP "llama-win"
Invoke-WebRequest $asset.browser_download_url -OutFile $zip -Headers $ghHeaders
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
Expand-Archive $zip -DestinationPath $out

$cli = Get-ChildItem -Recurse -Path $out -Filter "llama-cli.exe" | Select-Object -First 1
if (-not $cli) { throw "llama-cli.exe introuvable dans l'archive llama.cpp." }
Copy-Item $cli.FullName "$BinDir/llama-completion-$Triple.exe"
# llama-server.exe (même archive, mêmes DLL) : serveur persistant + embeddings.
$srv = Get-ChildItem -Recurse -Path $out -Filter "llama-server.exe" | Select-Object -First 1
if (-not $srv) { throw "llama-server.exe introuvable dans l'archive llama.cpp." }
Copy-Item $srv.FullName "$BinDir/llama-server-$Triple.exe"
# DLL runtime (ggml*, llama…) à poser à côté de l'exe via resources (glob *.dll).
Get-ChildItem -Path $cli.DirectoryName -Filter "*.dll" | ForEach-Object {
    Copy-Item $_.FullName (Join-Path $LibDir $_.Name)
}

Write-Host "── 2/3 poppler (pdftotext + pdftoppm, release oschwartz10612/poppler-windows)"
$rel = Invoke-RestMethod "https://api.github.com/repos/oschwartz10612/poppler-windows/releases/latest" -Headers $ghHeaders
$asset = $rel.assets | Where-Object { $_.name -like "Release-*.zip" } | Select-Object -First 1
if (-not $asset) { throw "Aucun asset Release-*.zip dans la dernière release poppler-windows." }
Write-Host "   → $($asset.name)"
$zip = Join-Path $env:TEMP $asset.name
$out = Join-Path $env:TEMP "poppler-win"
Invoke-WebRequest $asset.browser_download_url -OutFile $zip -Headers $ghHeaders
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
Expand-Archive $zip -DestinationPath $out
$pdftotext = Get-ChildItem -Recurse -Path $out -Filter "pdftotext.exe" | Select-Object -First 1
if (-not $pdftotext) { throw "pdftotext.exe introuvable dans l'archive poppler." }
# L'exe, son binôme pdftoppm et TOUTES les DLL du même dossier (fermeture des deps).
foreach ($f in @("pdftotext.exe", "pdftoppm.exe")) {
    Copy-Item (Join-Path $pdftotext.DirectoryName $f) $PopDir
}
Get-ChildItem -Path $pdftotext.DirectoryName -Filter "*.dll" | Copy-Item -Destination $PopDir

Write-Host "── 3/3 tesseract + tessdata (fra+eng+osd)"
# Installeur UB Mannheim (NSIS) extrait via 7-Zip — pas d'archive portable officielle.
$7z = @("7z", "$env:ProgramFiles\7-Zip\7z.exe", "${env:ProgramFiles(x86)}\7-Zip\7z.exe") |
    Where-Object { Get-Command $_ -ErrorAction SilentlyContinue } | Select-Object -First 1
if ($7z) {
    # Pas d'URL épinglée : on scrape l'index UB Mannheim et on prend le dernier
    # installeur (le miroir retire parfois d'anciennes versions → 404 sinon).
    # ponytail: tri lexical — suffisant tant que tesseract ne passe pas en 5.10+.
    $idxUrl = "https://digi.bib.uni-mannheim.de/tesseract/"
    $idx = Invoke-WebRequest $idxUrl -UseBasicParsing
    $name = $idx.Links.href |
        Where-Object { $_ -match '^tesseract-ocr-w64-setup-[0-9].*\.exe$' } |
        Sort-Object | Select-Object -Last 1
    if (-not $name) { throw "Aucun installeur tesseract-ocr-w64-setup-*.exe trouvé sur $idxUrl" }
    Write-Host "   → $name"
    $setupUrl = "$idxUrl$name"
    $setup = Join-Path $env:TEMP "tesseract-setup.exe"
    $out = Join-Path $env:TEMP "tesseract-win"
    Invoke-WebRequest $setupUrl -OutFile $setup
    Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
    & $7z x $setup "-o$out" -y | Out-Null
    $tess = Get-ChildItem -Recurse -Path $out -Filter "tesseract.exe" | Select-Object -First 1
    if (-not $tess) { throw "tesseract.exe introuvable après extraction de l'installeur." }
    Copy-Item $tess.FullName $TesDir
    Get-ChildItem -Path $tess.DirectoryName -Filter "*.dll" | Copy-Item -Destination $TesDir
} elseif (Get-Command tesseract -ErrorAction SilentlyContinue) {
    # Pas de 7-Zip mais une install système : on la copie.
    $tessCmd = (Get-Command tesseract).Source
    Copy-Item $tessCmd $TesDir
    Get-ChildItem -Path (Split-Path $tessCmd) -Filter "*.dll" | Copy-Item -Destination $TesDir
    Write-Host "   → copié depuis l'installation système ($tessCmd)"
} else {
    # Honnête (ADR-0015) : sans tesseract, les PDF scannés resteront sans OCR.
    Write-Warning "7-Zip et tesseract introuvables → OCR non embarqué (PDF scannés indisponibles)."
    Write-Warning "Installe 7-Zip (winget install 7zip.7zip) puis relance ce script."
}
# tessdata (fast) : léger et suffisant pour l'OCR de documents.
foreach ($lang in @("fra", "eng", "osd")) {
    Invoke-WebRequest "https://github.com/tesseract-ocr/tessdata_fast/raw/main/$lang.traineddata" `
        -OutFile (Join-Path $TdaDir "$lang.traineddata")
}

Write-Host ""
Write-Host "✅ Sidecars Windows prêts :"
Get-ChildItem $BinDir | ForEach-Object { Write-Host "   $($_.Name) $([math]::Round($_.Length/1MB,1))MB" }
Write-Host "   win-libs : $((Get-ChildItem $LibDir).Count) DLL"
Write-Host "   poppler : $((Get-ChildItem $PopDir).Count) fichiers · tesseract : $((Get-ChildItem $TesDir).Count) fichiers · tessdata : $((Get-ChildItem $TdaDir).Count) langues"
