export type WorkspaceTheme = "warm" | "neutral";

export const WORKSPACE_THEME_STORAGE_KEY = "biflow-workspace-theme-v1";

export function readWorkspaceTheme(): WorkspaceTheme {
  const stored = localStorage.getItem(WORKSPACE_THEME_STORAGE_KEY);
  return stored === "neutral" ? "neutral" : "warm";
}

export function writeWorkspaceTheme(theme: WorkspaceTheme): void {
  localStorage.setItem(WORKSPACE_THEME_STORAGE_KEY, theme);
}
