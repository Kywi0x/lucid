import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";

/**
 * Notification système native (macOS / Windows). Best-effort : vérifie la
 * permission, la demande au besoin, et n'échoue JAMAIS bruyamment si elle est
 * refusée — l'appelant n'a rien à gérer.
 *
 * ⚠️ Piège macOS en `tauri dev` : le binaire dev n'est pas un vrai bundle
 * `.app`, donc macOS attribue la notification au terminal parent (Terminal /
 * iTerm / VS Code), pas à Lucid. `isPermissionGranted()` peut renvoyer `true`
 * alors que rien ne s'affiche si les notifications sont coupées pour CE terminal.
 * → Autorise le terminal dans Réglages Système → Notifications. Le nom et
 * l'icône « Lucid » corrects n'apparaissent qu'après un `tauri build` (bundle
 * signé). Détaillé dans le README.
 */
export async function notify(title: string, body: string): Promise<void> {
  try {
    let granted = await isPermissionGranted();
    if (!granted) granted = (await requestPermission()) === "granted";
    if (granted) sendNotification({ title, body });
  } catch {
    /* pas grave, juste pas de notification cette fois */
  }
}
