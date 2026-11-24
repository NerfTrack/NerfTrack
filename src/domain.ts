export type Range = '1D' | '1W' | '1M' | '3M' | '6M';
export type Confidence = 'high' | 'medium' | 'low' | 'none';

export type AppStatusState =
  | 'connected'
  | 'detecting'
  | 'settling'
  | 'recalibrating'
  | 'unsupported'
  | 'needs_setup'
  | 'error';

export type NavKey = 'home' | 'setup' | 'diagnostics' | 'history' | 'settings';

export type UpdateStatus =
  | 'idle'
  | 'not-configured'
  | 'checking'
  | 'up-to-date'
  | 'available'
  | 'downloading'
  | 'installing'
  | 'failed';

export interface DiscoveryStatus {
  state: 'auto_detected' | 'selected' | 'missing' | 'unsupported' | 'redacted' | 'not_required';
  redactedLocation: string | null;
  message: string;
}

export interface AppStatus {
  state: AppStatusState;
  label: string;
  detail: string;
  integrationMode: 'cli' | 'gui' | 'unknown';
  accountState: 'authenticated' | 'unsupported' | 'unknown';
  connectionQuality: 'good' | 'degraded' | 'offline' | 'unknown';
  plan: string | null;
  resetAt: number | null;
  lastUpdatedAt: number | null;
  codexHome: DiscoveryStatus;
  codexExecutable: DiscoveryStatus;
  appServer: DiscoveryStatus;
  dataQuality: 'complete' | 'partial' | 'interrupted' | 'unknown';
}

export interface CurrentQuote {
  estimatedWeeklyValueUsd: number | null;
  changeValueUsd: number | null;
  changePercent: number | null;
  observedCostUsd: number | null;
  weeklyUsedPercent: number | null;
  resetAt: number | null;
  resetReason: string | null;
  status: 'valid' | 'pending' | 'empty' | 'unsupported' | 'error';
  algorithmVersion: string;
  confidence: Confidence;
  validObservationCount: number;
  percentageCoverage: number | null;
  pricingSource: string | null;
  modelStatus: string | null;
  note: string | null;
}

export interface HistoryPoint {
  timestamp: number;
  estimatedWeeklyValueUsd: number | null;
  rawEstimatedWeeklyValueUsd: number | null;
  observedCostUsd: number | null;
  weeklyUsedPercent: number | null;
  resetAt: number | null;
  resetReason: string | null;
  isFinalized: boolean;
  isHeartbeat: boolean;
  epoch: number | null;
  confidence: Confidence;
  percentageCoverage: number | null;
  /** True when the chart created this point between stored observations. */
  isSynthetic?: boolean;
  /** Conservative metadata for manual endpoint comparisons. */
  comparisonEligible?: boolean;
}

export interface RangeStatistics {
  range: Range;
  baselineEstimatedWeeklyValueUsd: number | null;
  baselineTimestamp: number | null;
  currentEstimatedWeeklyValueUsd: number | null;
  deltaValueUsd: number | null;
  deltaPercent: number | null;
  pointCount: number;
  partial: boolean;
}

export interface HistoryResponse {
  points: HistoryPoint[];
  statistics: RangeStatistics;
  bucket: 'raw' | '5m' | '30m' | '2h' | '4h';
}

export interface Annotation {
  id: string;
  timestamp: number;
  label: string;
  kind: 'reset' | 'diagnostic' | 'note';
}

export interface DiagnosticsSummary {
  totalEvents: number;
  pricedEvents: number;
  pendingEvents: number;
  rejectedEvents: number;
  unattributedEvents: number;
  partialLineRetries: number;
  monitoringGaps: number;
  hiddenResets: number;
  reasons: Array<{ reason: string; count: number }>;
  modelIds: string[];
  unpricedModelIds: string[];
  privacy: string;
}

export interface AdvancedSettings {
  refreshIntervalSeconds: number;
  reconciliationIntervalHours: number;
  monitoringGapMinutes: number;
  reducedMotion: boolean;
}

export interface AppSettings extends AdvancedSettings {
  locale: 'system' | 'en-US' | 'zh-CN' | 'zh-TW';
  appearance: 'dark';
  currency: 'USD';
  localOnly: true;
  telemetry: false;
  autoUpdater: false;
  starterPageSeen: boolean;
  lastSeenReleaseNotesVersion?: string;
  installationMarker: string;
  customPricing: Array<CustomPriceOverride>;
}

export interface CustomPriceOverride {
  modelId: string;
  alias: string | null;
  inputUsdPerMillion: number;
  cachedInputUsdPerMillion: number;
  outputUsdPerMillion: number;
}

export interface RedactedSelection {
  selected: boolean;
  status: DiscoveryStatus;
}

export interface UpdateCheckResult {
  currentVersion: string;
  latestVersion: string | null;
  updateAvailable: boolean;
  releaseUrl: string | null;
  assetName: string | null;
  assetUrl: string | null;
  message: string;
}

export interface DownloadedUpdate {
  version: string;
  assetName: string;
  path: string;
}

export interface InstallUpdateResult {
  message: string;
}

export interface UpdateState {
  status: UpdateStatus;
  currentVersion: string;
  latestVersion: string | null;
  releaseUrl: string | null;
  assetName: string | null;
  message: string;
}
