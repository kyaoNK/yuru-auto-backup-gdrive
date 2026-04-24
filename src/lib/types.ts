export interface JobSummary {
  copied: number;
  errors: number;
}

export interface Config {
  source: string | null;
  destination: string | null;
  scheduleTime: string;
  autoStart: boolean;
  lastRunAt: string | null;
  lastSummary: JobSummary | null;
}

export interface Status {
  running: boolean;
  lastRunAt: string | null;
  nextRunAt: string | null;
  lastSummary: JobSummary | null;
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
