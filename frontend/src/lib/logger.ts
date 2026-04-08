import { api } from "./tauri";

export async function logClient(scope: string, message: string, level = "info") {
  try {
    console[level === "error" ? "error" : "log"](`[${scope}] ${message}`);
    await api.recordClientLog(level, scope, message);
  } catch (error) {
    console.error("failed to persist client log", error);
  }
}
