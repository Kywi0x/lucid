# ─── Lucid — sidecars du bundle Windows (MVP : IA locale seule) ──────────────
# Prépare les binaires externes attendus par tauri.windows.conf.json :
#   - lucid_mcp.exe        (compilé depuis ce repo)
#   - llama-completion.exe (= llama-cli.exe de la release officielle llama.cpp, CPU)
#   + les DLL de llama.cpp dans resources/win-libs/ (posées à côté de l'exe).
# poppler/tesseract (PDF/OCR) sont différés sur Windows v1 — dégradent proprement.
#
# ⚠️ Ordre important : llama (exe + DLL) AVANT `cargo build --bin lucid_mcp`, car
# build.rs (tauri_build::build) valide l'existence des externalBin + resources dès
# qu'on compile le crate. lucid_mcp reçoit un placeholder le temps de se compiler.
#
# Usage (runner windows-latest ou PC Windows) : pwsh scripts/bundle-sidecars.ps1
$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true   # cargo & co. : exit≠0 => stop
Set-Location (Join-Path $PSScriptRoot "..")

$Triple = "x86_64-pc-windows-msvc"
$BinDir = "src-tauri/binaries"
$LibDir = "src-tauri/resources/win-libs"
Remove-Item -Recurse -Force $BinDir, $LibDir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $BinDir, $LibDir | Out-Null

# Auth GitHub API si dispo (évite le rate-limit anonyme sur les runners partagés).
$ghHeaders = @{ "User-Agent" = "lucid-ci" }
if ($env:GITHUB_TOKEN) { $ghHeaders["Authorization"] = "Bearer $($env:GITHUB_TOKEN)" }

Write-Host "── 1/2 llama-completion (release officielle llama.cpp, CPU x64)"
$rel = Invoke-RestMethod "https://api.github.com/repos/ggml-org/llama.cpp/releases/latest" -Headers $ghHeaders
$asset = $rel.assets | Where-Object { $_.name -like "*bin-win-cpu-x64.zip" } | Select-Object -First 1
if (-not $asset) {
    $asset = $rel.assets | Where-Object { $_.name -like "*bin-win*x64.zip" } | Select-Object -First 1
}
if (-not $asset) { throw "Aucun asset llama.cpp win-x64 trouvé dans la dernière release." }
Write-Host "   → $($asset.name)"

$zip = Join-Path $env:TEMP $asset.name
$out = Join-Path $env:TEMP "llama-win"
Invoke-WebRequest $asset.browser_download_url -OutFile $zip -Headers $ghHeaders
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
Expand-Archive $zip -DestinationPath $out

$cli = Get-ChildItem -Recurse -Path $out -Filter "llama-cli.exe" | Select-Object -First 1
if (-not $cli) { throw "llama-cli.exe introuvable dans l'archive llama.cpp." }
Copy-Item $cli.FullName "$BinDir/llama-completion-$Triple.exe"
# DLL runtime (ggml*, llama…) à poser à côté de l'exe via resources (glob *.dll).
Get-ChildItem -Path $cli.DirectoryName -Filter "*.dll" | ForEach-Object {
    Copy-Item $_.FullName (Join-Path $LibDir $_.Name)
}

Write-Host "── 2/2 lucid_mcp (release)"
# Placeholder : build.rs valide l'existence de tous les externalBin (dont lucid_mcp)
# pendant sa propre compilation. Le vrai binaire l'écrase juste après.
New-Item -ItemType File -Force "$BinDir/lucid_mcp-$Triple.exe" | Out-Null
Push-Location src-tauri
cargo build --release --bin lucid_mcp --quiet
Pop-Location
Copy-Item "src-tauri/target/release/lucid_mcp.exe" "$BinDir/lucid_mcp-$Triple.exe" -Force

Write-Host ""
Write-Host "✅ Sidecars Windows prêts :"
Get-ChildItem $BinDir | ForEach-Object { Write-Host "   $($_.Name) $([math]::Round($_.Length/1MB,1))MB" }
Write-Host "   win-libs : $((Get-ChildItem $LibDir).Count) DLL"
