import type {
  AdvancedSettings,
  Annotation,
  AppSettings,
  AppStatus,
  CurrentQuote,
  DiagnosticsSummary,
  HistoryResponse,
  Range,
} from '../domain';

const minute = 60_000;
const hour = 60 * minute;
const day = 24 * hour;

export const demoNow = Date.now();

const rangeDuration: Record<Range, number> = {
  '1D': day,
  '1W': 7 * day,
  '1M': 30 * day,
  '3M': 90 * day,
  '6M': 180 * day,
};

const pathForRange: Record<Range, string> = {
  '1D': '5m',
  '1W': '30m',
  '1M': '2h',
  '3M': '4h',
  '6M': '4h',
};

export const demoStatus: AppStatus = {
  state: 'connected',
  label: 'Connected',
  detail: 'Local Mode',
  integrationMode: 'cli',
  accountState: 'authenticated',
  connectionQuality: 'good',
  plan: 'ChatGPT Plus',
  resetAt: demoNow + 2 * day + 7 * hour,
  lastUpdatedAt: demoNow,
  codexHome: {
    state: 'auto_detected',
    redactedLocation: '<codex-data-directory>',
    message: 'Auto-detected',
  },
  codexExecutable: {
    state: 'auto_detected',
    redactedLocation: 'local path redacted',
    message: 'Auto-detected',
  },
  appServer: {
    state: 'unsupported',
    redactedLocation: 'CLI App Server',
    message: 'Unavailable: App Server supervision is not integrated',
  },
  dataQuality: 'complete',
};

export const demoQuote: CurrentQuote = {
  estimatedWeeklyValueUsd: 371.28,
  changeValueUsd: -18.42,
  changePercent: -4.73,
  observedCostUsd: 138.6,
  weeklyUsedPercent: 34,
  resetAt: demoStatus.resetAt,
  resetReason: 'scheduled_reset',
  status: 'valid',
  algorithmVersion: 'nerftrack-token-api-equivalent-v3',
  confidence: 'high',
  validObservationCount: 6,
  percentageCoverage: 34,
  pricingSource: 'models_dev',
  modelStatus: 'models_dev',
  note: 'Rolling median of cumulative weekly cost-per-percent estimates.',
};

export const demoAnnotations: Annotation[] = [
  {
    id: 'weekly-reset',
    timestamp: demoNow - 4.7 * day,
    label: 'Weekly window · scheduled reset',
    kind: 'reset',
  },
  {
    id: 'manual-reset',
    timestamp: demoNow - 2.15 * day,
    label: 'Weekly window · reported reset changed',
    kind: 'reset',
  },
];

export const demoDiagnostics: DiagnosticsSummary = {
  totalEvents: 846,
  pricedEvents: 812,
  pendingEvents: 34,
  rejectedEvents: 0,
  unattributedEvents: 0,
  partialLineRetries: 4,
  monitoringGaps: 0,
  hiddenResets: 0,
  reasons: [
    { reason: 'Unknown API price for model', count: 22 },
    { reason: 'Reported reset changed', count: 1 },
    { reason: 'Waiting for positive paired deltas', count: 11 },
  ],
  modelIds: ['gpt-5-codex', 'gpt-5-codex-mini'],
  unpricedModelIds: ['local-codex-preview'],
  privacy: 'Prompts, account identifiers, and full local paths are never stored or returned.',
};

export const defaultAdvancedSettings: AdvancedSettings = {
  refreshIntervalSeconds: 10,
  reconciliationIntervalHours: 1,
  monitoringGapMinutes: 5,
  reducedMotion: false,
};

export const demoSettings: AppSettings = {
  ...defaultAdvancedSettings,
  locale: 'system',
  appearance: 'dark',
  currency: 'USD',
  localOnly: true,
  telemetry: false,
  autoUpdater: false,
  starterPageSeen: true,
  installationMarker: '',
  customPricing: [],
};

export function getDemoHistory(range: Range): HistoryResponse {
  const total =
    range === '1D' ? 96 : range === '1W' ? 168 : range === '1M' ? 180 : range === '3M' ? 270 : 360;
  const duration = rangeDuration[range];
  const step = duration / Math.max(total - 1, 1);
  const resetIndex = Math.floor(total * 0.52);
  const points = Array.from({ length: total }, (_, index) => {
    const progress = index / Math.max(total - 1, 1);
    const wave = Math.sin(index * 0.36) * 1.8 + Math.sin(index * 0.11) * 2.7;
    const noise = ((index * 17) % 11) * 0.08;
    const trend = progress * -29;
    const estimatedValueUsd = Number((401 + trend + wave * 0.72 + noise * 0.16).toFixed(2));
    return {
      timestamp: demoNow - duration + index * step,
      estimatedWeeklyValueUsd: estimatedValueUsd,
      rawEstimatedWeeklyValueUsd: Number(
        (estimatedValueUsd + Math.sin(index * 0.91) * 6.4).toFixed(2),
      ),
      observedCostUsd: Number((progress * 138.6).toFixed(2)),
      weeklyUsedPercent: Math.max(4, Number((11 + progress * 23).toFixed(1))),
      resetAt: index >= resetIndex ? Date.UTC(2026, 4, 14) : Date.UTC(2026, 4, 7),
      resetReason: index === resetIndex ? 'reported_reset_changed' : 'scheduled_reset',
      isFinalized: index < total - 2,
      isHeartbeat: index % 10 === 0,
      epoch: index < resetIndex ? 1 : 2,
      confidence: index < 8 ? ('low' as const) : ('high' as const),
      percentageCoverage: Number((progress * 34).toFixed(1)),
    };
  }).filter((_, index) => {
    if (range === '1D') return true;
    const progress = index / Math.max(total - 1, 1);
    return !((progress > 0.17 && progress < 0.33) || (progress > 0.61 && progress < 0.76));
  });
  points[points.length - 1].estimatedWeeklyValueUsd = demoQuote.estimatedWeeklyValueUsd ?? 0;
  return {
    points,
    statistics: {
      range,
      baselineEstimatedWeeklyValueUsd: points[0].estimatedWeeklyValueUsd,
      baselineTimestamp: points[0].timestamp,
      currentEstimatedWeeklyValueUsd: demoQuote.estimatedWeeklyValueUsd,
      deltaValueUsd: demoQuote.changeValueUsd,
      deltaPercent: demoQuote.changePercent,
      pointCount: points.length,
      partial: range !== '1D',
    },
    bucket: pathForRange[range] as HistoryResponse['bucket'],
  };
}
