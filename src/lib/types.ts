export interface JobSummary {
  copied: number;
  errors: number;
}

export interface JobFailure {
  at: string;
  message: string;
}

export interface DeletionPreview {
  checkedAt: string;
  candidates: string[];
  retained: string[];
  errors: [string, string][];
  orphans: OrphanBackup[];
}

export interface OrphanBackup {
  name: string;
  backup: string;
  source: string;
  missingSince: string | null;
  eligibleAt: string | null;
  reason: string;
  token: string | null;
}

export interface Config {
  source: string | null;
  destination: string | null;
  scheduleTime: string;
  autoStart: boolean;
  excludedFolders: string[];
  excludedFolderNames: string[];
  lastRunAt: string | null;
  lastSummary: JobSummary | null;
  lastError: JobFailure | null;
}

export interface Status {
  serviceError: string | null;
  running: boolean;
  lastRunAt: string | null;
  nextRunAt: string | null;
  lastSummary: JobSummary | null;
  lastError: JobFailure | null;
  source: string | null;
  destination: string | null;
  scheduleTime: string;
  autoStart: boolean;
}

export type DetectionSource = "registry" | "driveLetter" | "conventional";

export interface DriveCandidate {
  path: string;
  label: string;
  source: DetectionSource;
}
