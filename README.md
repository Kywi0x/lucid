# Tauri + React + Typescript

This template should help get you started developing with Tauri, React and Typescript in Vite.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## Notifications système (macOS) — piège en `tauri dev`

Lucid envoie des notifications natives (`tauri-plugin-notification`, helper
`src/lib/notify.ts`, bouton de test dans Réglages → Compte).

⚠️ **En `tauri dev`, le binaire n'est pas un vrai bundle `.app`** : macOS attribue
la notification au **terminal parent** (Terminal, iTerm, VS Code…), pas à Lucid.
Conséquences :

- `isPermissionGranted()` peut renvoyer `true` **sans que rien ne s'affiche** si
  les notifications sont coupées pour ce terminal.
- Le **nom** et l'**icône** « Lucid » corrects n'apparaissent qu'après un
  `tauri build` (bundle signé).

**Pour tester en dev** : Réglages Système → Notifications → sélectionne ton
terminal (Terminal / iTerm / VS Code) → active « Autoriser les notifications ».
Puis clique « Tester » dans Réglages → Compte.

## Icône barre de menu (tray, macOS)

Câblée côté Rust dans `setup()` (`setup_tray`, `src-tauri/src/lib.rs`) :
`TrayIconBuilder` + menu **Afficher / Quitter**. L'icône
(`src-tauri/icons/tray.png`, template monochrome 44×44) est en `icon_as_template`
→ elle suit automatiquement le thème clair/sombre du système. Régénérable via
`python3 src-tauri/icons/gen_tray.py src-tauri/icons/tray.png` (stdlib only, sans
PIL). Fonctionne en `dev` comme en `build`.
