// Tauri IPC 封装
import { invoke } from "@tauri-apps/api/core"
import type {
  CategoryResult,
  CleanResult,
  DirDetail,
  DriveInfo,
  DupeScanResult,
  EnvInfo,
  InstalledApp,
  LargeDeleteOutcome,
  LargeScanResult,
  RecoveryEntry,
  ResidueDir,
  ResidueInput,
  ScanResultDto,
  Settings,
  SpaceScanResult,
  Stats,
} from "./types"

export const api = {
  listDrives: () => invoke<DriveInfo[]>("list_drives"),
  getEnvInfo: () => invoke<EnvInfo>("get_env_info"),
  startScan: (drive: string) => invoke<number>("start_scan", { drive }),
  cancelScan: (sessionId: number) => invoke<void>("cancel_scan", { sessionId }),
  getScanResult: (sessionId: number) => invoke<ScanResultDto>("get_scan_result", { sessionId }),
  clean: (sessionId: number, categoryIds: string[]) =>
    invoke<CleanResult>("clean", { sessionId, categoryIds }),
  listRecovery: () => invoke<RecoveryEntry[]>("list_recovery"),
  restoreEntries: (entryIds: string[]) => invoke<number>("restore_entries", { entryIds }),
  deleteRecovery: (entryIds: string[]) => invoke<number>("delete_recovery", { entryIds }),
  clearRecovery: () => invoke<number>("clear_recovery"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  getStats: () => invoke<Stats>("get_stats"),
  // M2：大文件
  startLargeScan: (drive: string, thresholdMb: number) =>
    invoke<number>("start_large_scan", { drive, thresholdMb }),
  cancelLargeScan: (sessionId: number) => invoke<void>("cancel_large_scan", { sessionId }),
  getLargeScanResult: (sessionId: number) =>
    invoke<LargeScanResult>("get_large_scan_result", { sessionId }),
  deleteLargeFile: (path: string, allowPermanent: boolean) =>
    invoke<LargeDeleteOutcome>("delete_large_file", { path, allowPermanent }),
  addIgnoredFile: (path: string) => invoke<Settings>("add_ignored_file", { path }),
  clearIgnoredFiles: () => invoke<Settings>("clear_ignored_files"),
  // M2：重复文件
  startDupeScan: (drive: string) => invoke<number>("start_dupe_scan", { drive }),
  cancelDupeScan: (sessionId: number) => invoke<void>("cancel_dupe_scan", { sessionId }),
  getDupeScanResult: (sessionId: number) =>
    invoke<DupeScanResult>("get_dupe_scan_result", { sessionId }),
  deleteDupeFiles: (paths: string[]) => invoke<CleanResult>("delete_dupe_files", { paths }),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  // M3：应用卸载
  listInstalledApps: () => invoke<InstalledApp[]>("list_installed_apps"),
  launchUninstall: (app: { name: string; uninstallString: string; isUwp: boolean; id: string }) =>
    invoke<void>("launch_uninstall", {
      uninstallString: app.uninstallString,
      displayName: app.name,
      packageFullName: app.isUwp ? app.id.replace(/^uwp:/, "") : "",
      isUwp: app.isUwp,
    }),
  scanResidues: (displayName: string, publisher: string, installLocation: string) =>
    invoke<ResidueDir[]>("scan_residues", { displayName, publisher, installLocation }),
  cleanResidues: (dirs: ResidueInput[]) => invoke<CleanResult>("clean_residues", { dirs }),
  // M3：空间分析
  startSpaceScan: (drive: string) => invoke<number>("start_space_scan", { drive }),
  cancelSpaceScan: (sessionId: number) => invoke<void>("cancel_space_scan", { sessionId }),
  getSpaceScanResult: (sessionId: number) =>
    invoke<SpaceScanResult>("get_space_scan_result", { sessionId }),
  getDirDetail: (sessionId: number, path: string) =>
    invoke<DirDetail>("get_dir_detail", { sessionId, path }),
  deleteDirToRecovery: (path: string) => invoke<CleanResult>("delete_dir_to_recovery", { path }),
}

export type {
  CategoryResult,
  CleanResult,
  DirDetail,
  DriveInfo,
  DupeScanResult,
  EnvInfo,
  InstalledApp,
  LargeDeleteOutcome,
  LargeScanResult,
  RecoveryEntry,
  ResidueDir,
  ResidueInput,
  ScanResultDto,
  Settings,
  SpaceScanResult,
  Stats,
}
