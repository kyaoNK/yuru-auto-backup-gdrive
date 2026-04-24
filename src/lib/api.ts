import { invoke } from "@tauri-apps/api/core";
import type { Config, DriveCandidate, Status } from "./types";

export const api = {
  getConfig: () => invoke<Config>("get_config"),
  updateConfig: (config: Config) => invoke<void>("update_config", { config }),
  pickFolder: (startDir?: string) =>
    invoke<string | null>("pick_folder", { startDir: startDir ?? null }),
  detectDriveRoots: () => invoke<DriveCandidate[]>("detect_drive_roots"),
  getStatus: () => invoke<Status>("get_status"),
  runNow: () => invoke<void>("run_now"),
  listRecentLogs: (limit = 200) =>
    invoke<string[]>("list_recent_logs", { limit }),
  openAppDir: () => invoke<void>("open_app_dir"),
};
