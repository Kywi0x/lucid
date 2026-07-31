; Hooks NSIS — branchés via `bundle.windows.nsis.installerHooks` (tauri.windows.conf.json).
;
; Problème résolu ici — remonté par Liam le 2026-07-31 sur un PC Windows, pendant
; une mise à jour :
;
;   Error opening file for writing:
;   C:\Users\<user>\AppData\Local\Lucid\poppler\Lerc.dll
;
; NSIS ne peut pas écraser un fichier qu'un process tient ouvert. Or sous Windows
; les process ENFANTS ne meurent PAS avec leur parent (pas de process group comme
; sur Unix) : un `llama-server` ou un `pdftotext` lancé par Lucid survit à la
; fermeture de l'app et garde ses DLLs verrouillées. Le template Tauri ferme
; l'application principale, pas sa descendance — d'où ce hook, exécuté AVANT
; l'extraction des fichiers.
;
; Tué par nom d'image plutôt que par chemin : `wmic` (le seul moyen de filtrer sur
; le chemin de l'exécutable) est déprécié et absent des Windows récents. Le risque
; est nul en pratique — ce sont les binaires que Lucid embarque, et des CLI sans
; état : les interrompre ne perd aucune donnée, le pire cas est une extraction PDF
; à refaire.
;
; Best-effort : `taskkill` renvoie une erreur si l'image n'existe pas (cas normal),
; on ne la vérifie pas — l'installation ne doit jamais échouer pour ça.

!macro KillLucidSidecars
  DetailPrint "Fermeture des process Lucid restés ouverts…"
  nsExec::Exec 'taskkill /F /T /IM llama-server.exe'
  nsExec::Exec 'taskkill /F /T /IM llama-completion.exe'
  nsExec::Exec 'taskkill /F /T /IM pdftotext.exe'
  nsExec::Exec 'taskkill /F /T /IM pdftoppm.exe'
  nsExec::Exec 'taskkill /F /T /IM tesseract.exe'
!macroend

!macro NSIS_HOOK_PREINSTALL_RUN
  !insertmacro KillLucidSidecars
!macroend

; Même raison à la désinstallation : un sidecar encore vivant laisse un dossier
; poppler/ ou tesseract/ à moitié supprimé, et la réinstallation suivante retombe
; sur la même erreur d'écriture.
!macro NSIS_HOOK_PREUNINSTALL_RUN
  !insertmacro KillLucidSidecars
!macroend
