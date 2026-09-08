export interface CommandError {
  code: string;
  message: string;
  details?: Record<string, unknown> | null;
}

export type WorkflowState =
  | 'DRAFT'
  | 'ARCHITECTING'
  | 'READY_TO_FREEZE'
  | 'FROZEN'
  | 'BUILDING'
  | 'VALIDATING'
  | 'WAITING_FOR_REVIEW'
  | 'CORRECTIONS_REQUIRED'
  | 'BLOCKED'
  | 'ARCHITECTURE_CONCERN'
  | 'REVIEW_ACCEPTED'
  | 'FINAL_VALIDATION'
  | 'READY_FOR_HUMAN_REVIEW'
  | 'HUMAN_ACCEPTED'
  | 'ARCHITECTURE_CHANGE'
  | 'ARCHITECTING_REVISION'
  | 'READY_TO_REFREEZE'
  | 'PAUSED'
  | 'INTERRUPTED';

export interface GitStatusCounts {
  staged: number;
  unstaged: number;
  untracked: number;
  is_clean: boolean;
}

export interface GitRepoInfo {
  git_version: string;
  is_repo: boolean;
  root_dir?: string | null;
  current_branch?: string | null;
  is_detached?: boolean;
  head_commit?: string | null;
  status: GitStatusCounts;
  diff_summary: string;
}

export interface ProjectYaml {
  schema_version: number;
  project_id: string;
  name: string;
  current_architecture_version?: string | null;
  architecture_state: 'draft' | 'frozen';
  created_at: string;
}

export interface ProjectRecord {
  project_id: string;
  name: string;
  repository_path: string;
  created_at: string;
  updated_at: string;
  last_opened_at: string;
}

export interface ProjectSummary {
  project_id: string;
  name: string;
  repository_path: string;
  workflow_state?: WorkflowState | null;
  git_branch?: string | null;
  is_clean?: boolean | null;
  last_opened_at: string;
  is_available: boolean;
}

export interface WorkflowStateRecord {
  project_id: string;
  state: WorkflowState;
  resume_state?: WorkflowState | null;
  revision: number;
  updated_at: string;
}

export interface ProjectDetails {
  project: ProjectRecord;
  workflow_state: WorkflowStateRecord;
  artifact: ProjectYaml;
  git?: GitRepoInfo | null;
  is_available: boolean;
}

export interface ActivityEventRecord {
  id: number;
  project_id: string;
  timestamp: string;
  event_type: string;
  actor: string;
  summary: string;
  metadata_json: string;
}

// Phase 0 Diagnostics Types
export interface ModelInfo {
  id: string;
  name: string;
}

export interface SystemDiagnosticInfo {
  current_dir: string;
  git: GitRepoInfo | null;
  agy_detected: boolean;
  agy_path: string | null;
  agy_version: string | null;
  agy_models: ModelInfo[];
  agy_error: string | null;
}

export interface ProofResult {
  applied_migrations: Array<{ version: number; name: string; applied_at: string }>;
  test_record_id: number;
  test_record_message: string;
  total_records: number;
}

export interface AgyEvent {
  event: string;
  conversation_id?: string;
  [key: string]: unknown;
}

export interface BuilderTurnResponse {
  conversation_id: string;
  success: boolean;
  exit_code: number | null;
  stdout: string;
  stderr: string;
  duration_ms: number;
  events: AgyEvent[];
}
