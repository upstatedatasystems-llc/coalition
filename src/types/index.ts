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
  artifact?: ProjectYaml | null;
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
  conversation_id?: string | null;
  status: string;
  provider_status?: string | null;
  text_response: string;
  cumulative_usage: AgyUsage;
  was_canceled: boolean;
  stderr: string;
  has_blocked_actions?: boolean;
}

// Stage 2A ChatGPT Relay & Architecture Workspace Types
export type RelayRole = 'ARCHITECT' | 'REVIEWER';

export type RelayPacketType =
  | 'ARCHITECT_INITIAL'
  | 'ARCHITECT_UPDATE'
  | 'ARCHITECTURE_REVISION'
  | 'REVIEW_REQUEST'
  | 'REVIEW_VERDICT'
  | 'ARCHITECTURE_CONCERN_ANALYSIS';

export type RelayExpectedResponse = 'ARCHITECT_UPDATE' | 'REVIEW_VERDICT';

export interface RelayPacketMetadata {
  schema: number;
  project_id: string;
  packet_id: string;
  role: RelayRole;
  packet_type: RelayPacketType;
  architecture_version: string;
  expected_response: RelayExpectedResponse;
  created_at: string;
}

export interface RelayPacket {
  metadata: RelayPacketMetadata;
  prompt: string;
  human_instructions: string;
  project_context_summary: string;
}

export type ArtifactAction = 'CREATE' | 'MODIFY' | 'DELETE';
export type ArtifactDiffStatus = 'NEW' | 'MODIFIED' | 'UNCHANGED' | 'DELETED';

export interface ArtifactPreviewItem {
  path: string;
  title: string;
  action: ArtifactAction;
  status: ArtifactDiffStatus;
  current_content?: string | null;
  proposed_content: string;
  baseline_fingerprint?: string | null;
  baseline_exists?: boolean;
}

export type OpenQuestionStatus = 'OPEN' | 'RESOLVED';

export interface ProposedOpenQuestion {
  id: string;
  question: string;
  status: OpenQuestionStatus;
  resolution?: string | null;
}

export interface ImportPreview {
  import_id: string;
  packet_id: string;
  project_id: string;
  summary: string;
  artifacts: ArtifactPreviewItem[];
  open_questions: ProposedOpenQuestion[];
  raw_response: string;
}

export type ArtifactReadinessStatus = 'MISSING' | 'INCOMPLETE' | 'READY';
export type OverallReadiness = 'INCOMPLETE' | 'READY_TO_FREEZE';
export type ArtifactApplicability = 'REQUIRED' | 'OPTIONAL' | 'NOT_APPLICABLE';

export interface ArtifactReadinessItem {
  path: string;
  title: string;
  applicability?: ArtifactApplicability;
  status: ArtifactReadinessStatus;
  character_count: number;
  details?: string | null;
}

export interface ReadinessReport {
  policy_version?: number;
  overall_readiness: OverallReadiness;
  ready_required_count?: number;
  total_required_count?: number;
  total_artifacts_count?: number;
  artifacts: ArtifactReadinessItem[];
  has_open_questions: boolean;
  unresolved_open_questions_count?: number;
  // Deprecated / backwards compatibility aliases
  ready_count?: number;
  total_required?: number;
}

export interface ArtifactContentDetails {
  path: string;
  content: string;
  fingerprint?: string | null;
  exists: boolean;
}

export interface RelayHistoryItem {
  id: string;
  packet_id?: string | null;
  project_id: string;
  item_type: string;
  summary: string;
  status: string;
  created_at: string;
  error_message?: string | null;
}

export interface WorkspaceState {
  pending_packet?: RelayPacket | null;
  pending_preview?: ImportPreview | null;
  readiness: ReadinessReport;
  history: RelayHistoryItem[];
}

// Stage 2B Architecture Freeze & Git Boundaries Types
export interface FrozenGitBoundary {
  head_commit: string;
  branch: string | null;
  is_detached: boolean;
  is_clean: boolean;
  staged_count: number;
  unstaged_count: number;
  untracked_count: number;
  dirty_fingerprint: string;
  porcelain_status: string;
}

export interface BuilderPacketSummary {
  total_artifacts: number;
  total_characters: number;
  estimated_tokens: number;
  is_truncated: boolean;
}

export interface FreezePreview {
  preview_id: string;
  project_id: string;
  target_version: string;
  readiness_policy_version: number;
  ready_required_count: number;
  total_required_count: number;
  unresolved_open_questions_count: number;
  artifact_baselines: Record<string, string>;
  git_boundary: FrozenGitBoundary;
  builder_packet_summary: BuilderPacketSummary;
  created_at: string;
}

export interface BuilderPacketMetadata {
  schema_version: number;
  packet_id: string;
  project_id: string;
  project_name: string;
  architecture_version: string;
  builder_epoch_id: string;
  created_at: string;
  contract_fingerprint: string;
  git_head_commit: string;
  git_branch: string | null;
}

export interface BuilderPacketArtifact {
  path: string;
  title: string;
  content: string;
  fingerprint: string;
}

export interface BuilderPacket {
  metadata: BuilderPacketMetadata;
  summary: string;
  builder_rules: string;
  artifacts: BuilderPacketArtifact[];
  is_truncated: boolean;
  prompt: string;
}

export interface FreezeResult {
  project_id: string;
  architecture_version: string;
  epoch_id: string;
  frozen_at: string;
  manifest_fingerprint: string;
  git_boundary: FrozenGitBoundary;
  snapshot_path: string;
  builder_packet: BuilderPacket;
}

export type DriftType = 'MODIFIED' | 'DELETED' | 'ADDED';

export interface DriftedArtifact {
  path: string;
  drift_type: DriftType;
  frozen_fingerprint: string | null;
  active_fingerprint: string | null;
}

export interface DriftReport {
  has_drift: boolean;
  is_frozen: boolean;
  architecture_version: string | null;
  drifted_artifacts: DriftedArtifact[];
  checked_at: string;
}

export interface DriftDiff {
  path: string;
  drift_type: DriftType;
  frozen_content: string | null;
  active_content: string | null;
}

// Stage 3 Builder Control Plane Types
export interface AgyUsage {
  input_tokens: number;
  output_tokens: number;
  thinking_tokens: number;
  cache_read_tokens: number;
  total_tokens: number;
}

export interface BuilderSessionRecord {
  session_id: string;
  project_id: string;
  epoch_id: string;
  conversation_id?: string | null;
  model: string;
  effort?: string | null;
  icarus_mode: boolean;
  status: 'RUNNING' | 'SUCCESS' | 'INTERRUPTED' | 'CANCELLED' | 'FAILED' | 'TIMEOUT';
  prompt: string;
  response_text?: string | null;
  error_message?: string | null;
  started_at: string;
  completed_at?: string | null;
  duration_ms: number;
  usage: AgyUsage;
}

export interface BuilderEventRecord {
  id: number;
  session_id: string;
  project_id: string;
  step_index?: number | null;
  event_type: string;
  state?: string | null;
  content?: string | null;
  details_json?: string | null;
  timestamp: string;
}

export interface ChatGptUsageSummary {
  rolling_5h_tokens: number;
  rolling_7d_tokens: number;
  total_tokens: number;
  total_packets_sent: number;
  total_imports_received: number;
  last_calibrated_at?: string | null;
  disclaimer: string;
  estimator_version: number;
  chars_per_token: number;
  sample_count: number;
  estimated_5h_capacity_pct?: number | null;
  estimated_weekly_capacity_pct?: number | null;
}

export interface UsageTelemetryReport {
  provider_antigravity_usage: AgyUsage;
  active_model?: string | null;
  active_effort?: string | null;
  chatgpt_estimated_usage: ChatGptUsageSummary;
}

export interface PermissionRecord {
  id: number;
  project_id: string;
  session_id?: string | null;
  tool_name: string;
  target?: string | null;
  risk_level: string;
  decision: string;
  reason?: string | null;
  created_at: string;
}

export interface IcarusState {
  project_id: string;
  enabled: boolean;
  enabled_at?: string | null;
  enabled_by?: string | null;
}

export interface StartBuilderTurnPayload {
  projectId: string;
  model?: string | null;
  effort?: string | null;
}

