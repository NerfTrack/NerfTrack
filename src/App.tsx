import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type {
  AppSettings,
  CustomPriceOverride,
  AppStatus,
  CurrentQuote,
  HistoryPoint,
  NavKey,
  Range,
} from './domain';
import { DiagnosticsView } from './components/DiagnosticsView';
import { Icon } from './components/Icons';
import { HistoryView } from './components/HistoryView';
import { MetricCard, UsageRing } from './components/MetricCard';
import { SetupView } from './components/SetupView';
import { SettingsView } from './components/SettingsView';
import { SideNav } from './components/SideNav';
import { StarterPage } from './components/StarterPage';
import { UsageChart } from './components/UsageChart';
import {
  getAnnotations,
  getCurrentQuote,
  getCurrentStatus,
  getDiagnosticsSummary,
  getHistory,
  getSettings,
  importAllData,
  resetAllData,
  resetAnnotations,
  restoreLastCheckpoint,
  retryDetection,
  selectCodexExecutable,
  selectCodexHome,
  updateSettings,
} from './lib/commands';
import { demoQuote, demoStatus } from './lib/fixtures';
import { GITHUB_REPOSITORY_URL, SHARE_GRAPH_DISCUSSION_URL } from './lib/config';
import { getChartEstimate } from './lib/comparison';
import {
  checkForUpdate,
  consumeUpdateFailure,
  CURRENT_APP_VERSION,
  downloadUpdate,
  initialUpdateState,
  installUpdate,
  openExternalUrl,
} from './lib/updater';
import type { Annotation, DiagnosticsSummary, HistoryResponse } from './domain';
import type { UpdateCheckResult, UpdateState } from './domain';
import {
  detectLocale,
  I18nProvider,
  translate,
  useI18n,
  type Locale,
  type MessageKey,
} from './i18n';

const ranges: Range[] = ['1D', '1W', '1M', '3M', '6M'];
const rangeLabelKeys: Record<Range, MessageKey> = {
  '1D': 'home.range.1D',
  '1W': 'home.range.1W',
  '1M': 'home.range.1M',
  '3M': 'home.range.3M',
  '6M': 'home.range.6M',
};
const rangeDurationMs: Record<Range, number> = {
  '1D': 86_400_000,
  '1W': 604_800_000,
  '1M': 2_592_000_000,
  '3M': 7_776_000_000,
  '6M': 15_552_000_000,
};

type Translate = (key: MessageKey, values?: Record<string, string | number>) => string;

function formatUsd(value: number | null, locale: Locale, t: Translate) {
  return value === null
    ? t('home.notAvailable')
    : `$${value.toLocaleString(locale, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

function formatEstimatedUsd(value: number | null, locale: Locale, t: Translate) {
  return value === null ? t('home.notAvailable') : `≈$${Math.round(value).toLocaleString(locale)}`;
}

function formatSignedUsd(value: number | null, locale: Locale) {
  if (value === null) return '—';
  return `${value < 0 ? '−' : '+'}$${Math.abs(value).toLocaleString(locale, {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })}`;
}

function formatPercent(value: number | null, locale: Locale) {
  return value === null
    ? '—'
    : `${value < 0 ? '−' : '+'}${Math.abs(value).toLocaleString(locale, {
        minimumFractionDigits: 2,
        maximumFractionDigits: 2,
        useGrouping: false,
      })}%`;
}

function hasStableEstimate(quote: CurrentQuote | null) {
  return (
    quote?.status === 'valid' && (quote.confidence === 'medium' || quote.confidence === 'high')
  );
}

function formatCoverage(value: number | null | undefined, locale: Locale, t: Translate) {
  if (value === null || value === undefined) return t('home.unknownCoverage');
  return t('home.coverage', {
    value: value.toLocaleString(locale, { maximumFractionDigits: 1 }),
  });
}

function formatObservationCount(count: number, locale: Locale, t: Translate) {
  return t(count === 1 ? 'home.validObservation' : 'home.validObservations', {
    count: count.toLocaleString(locale),
  });
}

function calibrationNote(quote: CurrentQuote | null, locale: Locale, t: Translate) {
  if (!quote || quote.estimatedWeeklyValueUsd === null) {
    return t('home.waitingForPair');
  }
  return t('home.earlyProjection', {
    value: formatEstimatedUsd(quote.estimatedWeeklyValueUsd, locale, t),
    observations: formatObservationCount(quote.validObservationCount, locale, t),
    coverage: formatCoverage(quote.percentageCoverage, locale, t),
  });
}

function formatReset(status: AppStatus, now: number, t: Translate) {
  if (!status.resetAt) return t('home.pending');
  const remaining = status.resetAt - now;
  if (remaining <= 0) return t('home.resetObserved');
  const minutes = Math.max(1, Math.floor(remaining / 60_000));
  const days = Math.floor(minutes / 1_440);
  const hours = Math.floor((minutes % 1_440) / 60);
  const remainderMinutes = minutes % 60;
  if (days > 0)
    return `${days}${t('common.dayShort')} ${hours}${t('common.hourShort')} ${remainderMinutes}${t('common.minuteShort')}`;
  if (hours > 0)
    return `${hours}${t('common.hourShort')} ${remainderMinutes}${t('common.minuteShort')}`;
  return `${remainderMinutes}${t('common.minuteShort')}`;
}

function formatResetDate(timestamp: number | null, locale: Locale, t: Translate) {
  if (!timestamp) return t('home.awaitingWindow');
  return new Date(timestamp).toLocaleString(locale, {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}

function HeaderIcon() {
  return (
    <div className="hero-icon">
      <Icon name="terminal" size={33} strokeWidth={1.6} />
    </div>
  );
}

function useLiveNow() {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, []);
  return now;
}

function ResetMetric({ status }: { status: AppStatus }) {
  const now = useLiveNow();
  const { locale, t } = useI18n();
  return (
    <MetricCard
      icon="clock"
      iconTone="blue"
      label={t('home.resetsIn')}
      value={formatReset(status, now, t)}
      detail={formatResetDate(status.resetAt, locale, t)}
    />
  );
}

function LiveRefreshStatus() {
  const now = useLiveNow();
  const { locale, t } = useI18n();
  return (
    <span className="refresh-status" aria-live="off">
      <i />
      {t('home.live')} ·{' '}
      {new Date(now).toLocaleTimeString(locale, {
        hour: 'numeric',
        minute: '2-digit',
        second: '2-digit',
      })}
      {' · '}
      {t('home.dataInterval', { seconds: 10 })}
    </span>
  );
}

function LocalIndexingBanner() {
  const { t } = useI18n();
  return (
    <div className="indexing-banner" role="status" aria-live="polite">
      <span className="indexing-spinner" aria-hidden="true" />
      <span>
        <strong>{t('home.indexing')}</strong>
        <small>{t('home.indexingContinue')}</small>
      </span>
    </div>
  );
}

function updateStateFromResult(result: UpdateCheckResult): UpdateState {
  return {
    status: result.updateAvailable
      ? 'available'
      : !GITHUB_REPOSITORY_URL
        ? 'not-configured'
        : 'up-to-date',
    currentVersion: result.currentVersion,
    latestVersion: result.latestVersion,
    releaseUrl: result.releaseUrl,
    assetName: result.assetName,
    message: result.message,
  };
}

function errorMessage(cause: unknown) {
  return cause instanceof Error ? cause.message : String(cause);
}

function RangeSelector({ range, onChange }: { range: Range; onChange: (range: Range) => void }) {
  const { t } = useI18n();
  return (
    <div className="range-control" role="tablist" aria-label={t('home.historyRange')}>
      {ranges.map((item) => (
        <button
          key={item}
          className={range === item ? 'selected' : ''}
          onClick={() => onChange(item)}
          role="tab"
          aria-selected={range === item}
        >
          {item}
        </button>
      ))}
    </div>
  );
}

export function HomeView({
  status,
  quote,
  history,
  annotations,
  range,
  reducedMotion,
  isRefreshing,
  onRefresh,
  onRangeChange,
  onResetAnnotations,
  onShareGraph,
}: {
  status: AppStatus;
  quote: CurrentQuote | null;
  history: HistoryResponse;
  annotations: Annotation[];
  range: Range;
  reducedMotion: boolean;
  isRefreshing: boolean;
  onRefresh: () => void;
  onRangeChange: (range: Range) => void;
  onResetAnnotations: () => void;
  onShareGraph?: () => Promise<void>;
}) {
  const { locale, t } = useI18n();
  const [scrubbed, setScrubbed] = useState<{
    point: HistoryPoint;
    anchor: HistoryPoint | null;
  } | null>(null);
  const [isSharing, setIsSharing] = useState(false);
  const [shareError, setShareError] = useState<string | null>(null);
  const displayValue = scrubbed
    ? getChartEstimate(scrubbed.point)
    : (quote?.estimatedWeeklyValueUsd ?? null);
  const stableEstimate = hasStableEstimate(quote) && !scrubbed;
  const comparisonValue =
    stableEstimate || scrubbed
      ? scrubbed?.anchor
        ? getChartEstimate(scrubbed.anchor)
        : (history.statistics.baselineEstimatedWeeklyValueUsd ?? null)
      : null;
  const displayChange =
    displayValue !== null && comparisonValue !== null ? displayValue - comparisonValue : null;
  const displayPercent =
    displayChange !== null && comparisonValue ? (displayChange / comparisonValue) * 100 : null;
  const signalPoints = history.points.filter((point) => getChartEstimate(point) !== null);
  const availableHistoryStart = signalPoints[0]?.timestamp ?? null;
  const availableHistoryEnd = signalPoints.at(-1)?.timestamp ?? null;
  const usesAvailableHistory =
    availableHistoryStart !== null &&
    availableHistoryEnd !== null &&
    availableHistoryEnd - availableHistoryStart < rangeDurationMs[range] * 0.98;
  const comparisonStartTimestamp = history.statistics.baselineTimestamp ?? availableHistoryStart;
  const comparisonLabel = scrubbed?.anchor
    ? t('home.selectedRange')
    : scrubbed
      ? new Date(scrubbed.point.timestamp).toLocaleString(locale, {
          month: 'short',
          day: 'numeric',
          hour: range === '1D' ? 'numeric' : undefined,
          minute: range === '1D' ? '2-digit' : undefined,
        })
      : history.statistics.baselineEstimatedWeeklyValueUsd === null
        ? t('home.rangeUnavailable', { range: t(rangeLabelKeys[range]) })
        : (history.statistics.partial || usesAvailableHistory) && comparisonStartTimestamp !== null
          ? t('home.since', {
              date: new Date(comparisonStartTimestamp).toLocaleString(locale, {
                month: 'short',
                day: 'numeric',
                hour: range === '1D' ? 'numeric' : undefined,
                minute: range === '1D' ? '2-digit' : undefined,
              }),
            })
          : t(rangeLabelKeys[range]);
  const isEmpty = displayValue === null || !quote || quote.status === 'empty';
  const selectRange = (nextRange: Range) => {
    setScrubbed(null);
    onRangeChange(nextRange);
  };

  const shareGraph = async () => {
    if (!onShareGraph || isSharing) return;
    setShareError(null);
    setIsSharing(true);
    try {
      await onShareGraph();
    } catch (cause) {
      setShareError(t('home.shareFailed', { error: errorMessage(cause) }));
    } finally {
      setIsSharing(false);
    }
  };

  return (
    <section className="home-page page-shell">
      <header className="hero-heading">
        <div className="hero-title-wrap">
          <HeaderIcon />
          <div>
            <h1>{t('home.title')}</h1>
            <p>{stableEstimate ? t('home.stableDescription') : t('home.calibratingDescription')}</p>
          </div>
        </div>
        <div className="hero-controls">
          <RangeSelector range={range} onChange={selectRange} />
          <button
            className={`refresh-button ${isRefreshing ? 'is-refreshing' : ''}`}
            aria-label={t('home.refresh')}
            title={t('home.refresh')}
            disabled={isRefreshing}
            onClick={onRefresh}
          >
            <Icon name="refresh" size={19} />
          </button>
          <button
            className="share-graph-hero-button"
            type="button"
            onClick={() => void shareGraph()}
            disabled={!onShareGraph || isSharing}
            title={t('home.shareTitle')}
          >
            <Icon name="message" size={17} />
            <span>{isSharing ? t('home.opening') : t('home.shareGraph')}</span>
          </button>
        </div>
      </header>
      <div className="quote-heading">
        <strong className={isEmpty || (!stableEstimate && !scrubbed) ? 'empty-value' : ''}>
          {stableEstimate || scrubbed
            ? formatEstimatedUsd(displayValue, locale, t)
            : t('home.calibrating')}
        </strong>
        {!isEmpty && (stableEstimate || scrubbed) && (
          <p className={displayChange !== null && displayChange < 0 ? 'negative' : 'positive'}>
            {formatSignedUsd(displayChange, locale)}{' '}
            {displayPercent !== null ? `(${formatPercent(displayPercent, locale)})` : ''}{' '}
            <span>{comparisonLabel}</span>
          </p>
        )}
        {(isEmpty || (!stableEstimate && !scrubbed)) && (
          <p className="muted-state">{calibrationNote(quote, locale, t)}</p>
        )}
      </div>
      <div className="chart-panel">
        <UsageChart
          points={history.points}
          annotations={annotations}
          range={range}
          reducedMotion={reducedMotion}
          changeValueUsd={history.statistics.deltaValueUsd}
          baselineEstimatedWeeklyValueUsd={history.statistics.baselineEstimatedWeeklyValueUsd}
          onScrub={(point, anchor) => setScrubbed(point ? { point, anchor } : null)}
        />
        <div className="chart-actions">
          <span>
            {history.statistics.partial || usesAvailableHistory
              ? t('home.allHistory')
              : t('home.completeRange')}
          </span>
          <button type="button" onClick={onResetAnnotations}>
            <Icon name="refresh" size={14} />
            {t('home.resetAnnotations')}
          </button>
        </div>
        {shareError && (
          <p className="share-graph-error" role="alert">
            {shareError}
          </p>
        )}
      </div>
      <div className="metric-grid">
        <MetricCard
          icon="chart"
          iconTone="green"
          label={hasStableEstimate(quote) ? t('home.stableValue') : t('home.earlyValue')}
          value={formatEstimatedUsd(quote?.estimatedWeeklyValueUsd ?? null, locale, t)}
          detail={
            hasStableEstimate(quote)
              ? t('home.cumulativeEstimate')
              : `${t(`home.confidence.${quote?.confidence ?? 'none'}`)} · ${formatCoverage(
                  quote?.percentageCoverage,
                  locale,
                  t,
                )}`
          }
        />
        <MetricCard
          icon="chart"
          iconTone="lime"
          label={t('home.weeklyUsed')}
          value={
            quote?.weeklyUsedPercent === null || quote?.weeklyUsedPercent === undefined
              ? '—'
              : `${Math.round(quote.weeklyUsedPercent)}%`
          }
          detail={t('home.ofAllowance')}
        >
          <UsageRing value={quote?.weeklyUsedPercent ?? null} />
        </MetricCard>
        <ResetMetric status={status} />
        <MetricCard
          icon="activity"
          iconTone="purple"
          label={t('home.observedTokenCost')}
          value={formatUsd(quote?.observedCostUsd ?? null, locale, t)}
          detail={t('home.thisWindow')}
        />
        <MetricCard
          icon="shield"
          iconTone="blue"
          label={t('home.confidence')}
          value={
            quote?.status === 'valid'
              ? t(`home.confidence.${quote.confidence}`)
              : quote?.status === 'pending'
                ? t('home.pending')
                : t('home.unavailable')
          }
          detail={
            quote?.validObservationCount
              ? formatObservationCount(quote.validObservationCount, locale, t)
              : t('home.needPairedDeltas')
          }
        />
      </div>
      <footer className="app-footer">
        <span>
          <Icon name="info" size={16} />
          {t('home.footer')}
        </span>
        <LiveRefreshStatus />
      </footer>
    </section>
  );
}

export default function App() {
  const [active, setActive] = useState<NavKey>('home');
  const [range, setRange] = useState<Range>('1W');
  const [status, setStatus] = useState<AppStatus>(demoStatus);
  const [quote, setQuote] = useState<CurrentQuote | null>(demoQuote);
  const activeRange = useRef(range);
  const historyCache = useRef<Partial<Record<Range, HistoryResponse>>>({});
  const refreshInFlight = useRef(false);
  const [histories, setHistories] = useState<Partial<Record<Range, HistoryResponse>>>({});
  const history = histories[range] ?? null;
  const [annotations, setAnnotations] = useState<Annotation[]>([]);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsSummary | null>(null);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [loadError, setLoadError] = useState(false);
  const [starterPageVisible, setStarterPageVisible] = useState(false);
  const [updateState, setUpdateState] = useState<UpdateState>(initialUpdateState);
  const updateInFlight = useRef(false);

  const refresh = useCallback(async (requestedRange?: Range) => {
    if (refreshInFlight.current) return;
    refreshInFlight.current = true;
    setIsRefreshing(true);
    try {
      const historyRanges = requestedRange
        ? [requestedRange]
        : ranges.every((item) => historyCache.current[item])
          ? [activeRange.current]
          : ranges;
      // Settings are served from the startup cache, so load them before any
      // database-dependent reads. This keeps onboarding available while the
      // background worker performs a first migration/rebuild.
      const nextSettings = await getSettings();
      setSettings(nextSettings);
      // Status reconciliation imports newly written usage before every dependent read.
      // Sequencing it first prevents a refresh from mixing old chart data with new status.
      const nextStatus = await getCurrentStatus();
      // Startup migration and historical repricing run in the Rust background
      // worker. Show that state immediately instead of keeping the whole window
      // behind the initial data-read promise.
      setStatus(nextStatus);
      const [nextQuote, nextHistories, nextAnnotations, nextDiagnostics] = await Promise.all([
        getCurrentQuote(),
        Promise.all(historyRanges.map(async (item) => [item, await getHistory(item)] as const)),
        getAnnotations(),
        getDiagnosticsSummary(),
      ]);
      const historyUpdates = Object.fromEntries(nextHistories) as Partial<
        Record<Range, HistoryResponse>
      >;
      Object.assign(historyCache.current, historyUpdates);
      setQuote(nextQuote);
      setStatus(nextStatus);
      setHistories((current) => ({ ...current, ...historyUpdates }));
      setAnnotations(nextAnnotations);
      setDiagnostics(nextDiagnostics);
      setLoadError(false);
    } catch {
      setQuote(null);
      setAnnotations([]);
      setDiagnostics(null);
      setStatus((current) => ({
        ...current,
        state: 'error',
        label: 'Unavailable',
        detail: 'Local state error',
        connectionQuality: 'offline',
        dataQuality: 'interrupted',
      }));
      setLoadError(true);
    } finally {
      setIsLoading(false);
      setIsRefreshing(false);
      refreshInFlight.current = false;
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const checkForUpdates = useCallback(async () => {
    if (updateInFlight.current) return;
    updateInFlight.current = true;
    setUpdateState((current) => ({
      ...current,
      status: 'checking',
      message: 'Checking GitHub Releases for the latest NerfTrack build…',
    }));
    try {
      const [result, previousFailure] = await Promise.all([
        checkForUpdate(GITHUB_REPOSITORY_URL),
        consumeUpdateFailure().catch(() => null),
      ]);
      const nextState = updateStateFromResult(result);
      setUpdateState(
        previousFailure
          ? {
              ...nextState,
              status: 'failed',
              message: `Previous update attempt failed: ${previousFailure}`,
            }
          : nextState,
      );
    } catch (cause) {
      setUpdateState((current) => ({
        ...current,
        status: 'failed',
        message: errorMessage(cause),
      }));
    } finally {
      updateInFlight.current = false;
    }
  }, []);

  const handleUpdate = useCallback(async () => {
    if (updateInFlight.current) return;
    if (updateState.status !== 'available') {
      await checkForUpdates();
      return;
    }
    if (!updateState.assetName) {
      setUpdateState((current) => ({
        ...current,
        status: 'failed',
        message: 'A newer release exists, but it has no compatible Windows or macOS asset.',
      }));
      return;
    }
    updateInFlight.current = true;
    setUpdateState((current) => ({
      ...current,
      status: 'downloading',
      message: `Downloading ${current.assetName}…`,
    }));
    try {
      const downloaded = await downloadUpdate(GITHUB_REPOSITORY_URL);
      setUpdateState((current) => ({
        ...current,
        status: 'installing',
        latestVersion: downloaded.version,
        assetName: downloaded.assetName,
        message: 'Download complete. Applying the update and restarting NerfTrack…',
      }));
      const installed = await installUpdate(downloaded.path);
      setUpdateState((current) => ({
        ...current,
        status: 'installing',
        message: installed.message,
      }));
    } catch (cause) {
      setUpdateState((current) => ({
        ...current,
        status: 'failed',
        message: errorMessage(cause),
      }));
    } finally {
      updateInFlight.current = false;
    }
  }, [checkForUpdates, updateState]);

  useEffect(() => {
    void checkForUpdates();
  }, [checkForUpdates]);

  useEffect(() => {
    if (settings && !settings.starterPageSeen) setStarterPageVisible(true);
  }, [settings]);

  useEffect(() => {
    const timer = window.setInterval(
      () => void refresh(),
      (settings?.refreshIntervalSeconds ?? 10) * 1_000,
    );
    return () => window.clearInterval(timer);
  }, [refresh, settings?.refreshIntervalSeconds]);

  useEffect(() => {
    const refreshWhenVisible = () => {
      if (document.visibilityState !== 'hidden') void refresh();
    };
    window.addEventListener('focus', refreshWhenVisible);
    document.addEventListener('visibilitychange', refreshWhenVisible);
    return () => {
      window.removeEventListener('focus', refreshWhenVisible);
      document.removeEventListener('visibilitychange', refreshWhenVisible);
    };
  }, [refresh]);

  const handleRangeChange = (nextRange: Range) => {
    activeRange.current = nextRange;
    setRange(nextRange);
    void refresh(nextRange);
  };

  const handleSettingChange = async (key: keyof AppSettings, value: number | boolean | string) => {
    if (!settings) return;
    const nextSettings = { ...settings, [key]: value };
    setSettings(nextSettings);
    if (key in nextSettings) {
      try {
        await updateSettings(nextSettings);
      } catch {
        setSettings(settings);
        setLoadError(true);
      }
    }
  };

  const handleCustomPricingChange = async (customPricing: CustomPriceOverride[]) => {
    if (!settings) return;
    const nextSettings = { ...settings, customPricing };
    setSettings(nextSettings);
    try {
      await updateSettings(nextSettings);
      await refresh();
    } catch {
      setSettings(settings);
      setLoadError(true);
      throw new Error('save failed');
    }
  };

  const handleStarterPageComplete = async () => {
    if (!settings) return;
    const nextSettings = { ...settings, starterPageSeen: true };
    const savedSettings = await updateSettings(nextSettings);
    setSettings(savedSettings);
    setStarterPageVisible(false);
    void refresh();
  };

  const handleOpenStarterPage = () => setStarterPageVisible(true);

  const handleShareGraph = useCallback(async () => {
    await openExternalUrl(SHARE_GRAPH_DISCUSSION_URL);
  }, []);

  const handleResetAllData = async () => {
    await resetAllData();
    historyCache.current = {};
    setHistories({});
    setQuote(null);
    setAnnotations([]);
    setDiagnostics(null);
    await refresh();
  };

  const refreshAfterDataRestore = async (restore: () => Promise<void>) => {
    await restore();
    historyCache.current = {};
    setHistories({});
    await refresh();
  };

  const handleRestoreLastCheckpoint = () => refreshAfterDataRestore(restoreLastCheckpoint);

  const handleImportAllData = () => refreshAfterDataRestore(importAllData);

  const runDetection = async () => {
    setStatus((current) => ({
      ...current,
      state: 'detecting',
      label: 'Detecting',
      detail: 'Local Mode',
    }));
    try {
      const next = await retryDetection();
      setStatus(next);
      await refresh();
    } catch {
      setLoadError(true);
    }
  };

  const handleChooseHome = async () => {
    try {
      const selection = await selectCodexHome();
      if (selection.selected) {
        setStatus((current) => ({ ...current, codexHome: selection.status }));
        setStatus(await retryDetection());
        await refresh();
      }
    } catch {
      setLoadError(true);
    }
  };

  const handleChooseExecutable = async () => {
    try {
      const selection = await selectCodexExecutable();
      if (selection.selected) {
        setStatus((current) => ({ ...current, codexExecutable: selection.status }));
        setStatus(await retryDetection());
      }
    } catch {
      setLoadError(true);
    }
  };

  const displayHistory = useMemo(
    () =>
      history ?? {
        points: [],
        statistics: {
          range,
          baselineEstimatedWeeklyValueUsd: null,
          baselineTimestamp: null,
          currentEstimatedWeeklyValueUsd: null,
          deltaValueUsd: null,
          deltaPercent: null,
          pointCount: 0,
          partial: true,
        },
        bucket: 'raw' as const,
      },
    [history, range],
  );
  const displaySettings = settings ?? {
    refreshIntervalSeconds: 10,
    reconciliationIntervalHours: 1,
    monitoringGapMinutes: 5,
    reducedMotion: false,
    locale: 'system' as const,
    appearance: 'dark' as const,
    currency: 'USD' as const,
    localOnly: true as const,
    telemetry: false as const,
    autoUpdater: false as const,
    starterPageSeen: true,
    installationMarker: '',
    customPricing: [],
  };
  const locale: Locale =
    displaySettings.locale === 'system' ? detectLocale() : displaySettings.locale;

  const renderPage = () => {
    if (isLoading && !history)
      return (
        <div className="loading-state">
          <span className="loading-spinner" />
          {translate(locale, 'common.loading')}
        </div>
      );
    switch (active) {
      case 'setup':
        return (
          <SetupView
            status={status}
            settings={displaySettings}
            onChooseHome={handleChooseHome}
            onChooseExecutable={handleChooseExecutable}
            onRetry={runDetection}
            onSettingChange={handleSettingChange}
          />
        );
      case 'diagnostics':
        return (
          <DiagnosticsView
            diagnostics={
              diagnostics ?? {
                totalEvents: 0,
                pricedEvents: 0,
                pendingEvents: 0,
                rejectedEvents: 0,
                unattributedEvents: 0,
                partialLineRetries: 0,
                monitoringGaps: 0,
                hiddenResets: 0,
                reasons: [],
                modelIds: [],
                unpricedModelIds: [],
                privacy: 'Waiting for local data.',
              }
            }
          />
        );
      case 'history':
        return (
          <HistoryView history={displayHistory} range={range} onRangeChange={handleRangeChange} />
        );
      case 'settings':
        return (
          <SettingsView
            settings={displaySettings}
            detectedModelIds={diagnostics?.unpricedModelIds ?? []}
            onChange={handleSettingChange}
            onCustomPricingChange={handleCustomPricingChange}
            onResetAllData={handleResetAllData}
            onRestoreLastCheckpoint={handleRestoreLastCheckpoint}
            onImportAllData={handleImportAllData}
            onOpenStarterPage={handleOpenStarterPage}
          />
        );
      default:
        return (
          <HomeView
            status={status}
            quote={quote}
            history={displayHistory}
            annotations={annotations}
            range={range}
            reducedMotion={displaySettings.reducedMotion}
            isRefreshing={isRefreshing}
            onRefresh={() => void refresh()}
            onRangeChange={handleRangeChange}
            onShareGraph={handleShareGraph}
            onResetAnnotations={async () => {
              try {
                await resetAnnotations();
                setAnnotations([]);
              } catch {
                setLoadError(true);
              }
            }}
          />
        );
    }
  };

  if (starterPageVisible && settings) {
    return (
      <I18nProvider locale={locale}>
        <StarterPage
          version={updateState.currentVersion || CURRENT_APP_VERSION}
          onComplete={handleStarterPageComplete}
        />
      </I18nProvider>
    );
  }

  return (
    <I18nProvider locale={locale}>
      <div className="app-window" lang={locale}>
        <SideNav
          active={active}
          status={status}
          observedEventCount={diagnostics?.totalEvents ?? 0}
          onNavigate={setActive}
          updateState={updateState}
          onUpdate={() => void handleUpdate()}
        />
        <main className="app-content">
          {status.state === 'recalibrating' && <LocalIndexingBanner />}
          {loadError && (
            <div className="global-error" role="alert">
              {translate(locale, 'common.localError')}
            </div>
          )}
          {renderPage()}
        </main>
      </div>
    </I18nProvider>
  );
}
