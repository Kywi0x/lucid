import { Monitor, Sun, Moon } from "lucide-react";
import { useTheme, type ThemeMode } from "@/lib/theme";

const NEXT: Record<ThemeMode, ThemeMode> = {
  system: "light",
  light: "dark",
  dark: "system",
};
const ICON = { system: Monitor, light: Sun, dark: Moon } as const;
const LABEL = { system: "Système", light: "Clair", dark: "Sombre" } as const;

export function ThemeToggle() {
  const { mode, setMode } = useTheme();
  const Icon = ICON[mode];
  return (
    <button
      onClick={() => setMode(NEXT[mode])}
      title={`Thème : ${LABEL[mode]} (cliquer pour changer)`}
      className="flex items-center gap-1.5 rounded-md px-2 py-1 text-sm text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
    >
      <Icon className="size-4" />
    </button>
  );
}
