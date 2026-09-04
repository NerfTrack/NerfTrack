import type { HistoryResponse, Range } from '../domain';
import { formatResetReason, useI18n } from '../i18n';
import { Icon } from './Icons';

interface HistoryViewProps {
  history: HistoryResponse;
  range: Range;
  onRangeChange: (range: Range) => void;
}

const ranges: Range[] = ['1D', '1W', '1M', '3M', '6M'];

export function HistoryView({ history, range, onRangeChange }: HistoryViewProps) {
  const { locale, t } = useI18n();
  const recent = history.points.slice(-8).reverse();
  const currency = new Intl.NumberFormat(locale, { style: 'currency', currency: 'USD' });
  return (
    <section className="page-shell history-page">
      <header className="page-heading history-heading">
        <div>
          <h1>{t('history.title')}</h1>
          <p>{t('history.description')}</p>
        </div>
        <div className="range-tabs compact-tabs">
          {ranges.map((item) => (
            <button
              className={item === range ? 'selected' : ''}
              key={item}
              onClick={() => onRangeChange(item)}
            >
              {item}
            </button>
          ))}
        </div>
      </header>
      <div className="history-stats">
        <div>
          <span>{t('history.current')}</span>
          <strong>
            {history.statistics.currentEstimatedWeeklyValueUsd === null
              ? '—'
              : currency.format(history.statistics.currentEstimatedWeeklyValueUsd)}
          </strong>
        </div>
        <div>
          <span>{t('history.rangeChange')}</span>
          <strong
            className={
              history.statistics.deltaValueUsd !== null && history.statistics.deltaValueUsd < 0
                ? 'negative'
                : ''
            }
          >
            {history.statistics.deltaValueUsd === null
              ? '—'
              : `${history.statistics.deltaValueUsd < 0 ? '−' : '+'}${currency.format(Math.abs(history.statistics.deltaValueUsd))}`}
          </strong>
        </div>
        <div>
          <span>{t('history.observations')}</span>
          <strong>{history.statistics.pointCount.toLocaleString(locale)}</strong>
        </div>
        <div>
          <span>{t('history.bucket')}</span>
          <strong>{history.bucket === 'raw' ? t('history.bucketRaw') : history.bucket}</strong>
        </div>
      </div>
      <div className="panel history-table-panel">
        <div className="panel-heading">
          <Icon name="history" size={23} />
          <h2>{t('history.recent')}</h2>
          <span className="table-note">
            {history.statistics.partial ? t('history.allAvailable') : t('history.completeRange')}
          </span>
        </div>
        <div className="history-table" role="table" aria-label={t('history.recent')}>
          <div className="history-table-row history-table-header" role="row">
            <span>{t('history.date')}</span>
            <span>{t('history.estimatedValue')}</span>
            <span>{t('history.observedCost')}</span>
            <span>{t('history.weeklyUsage')}</span>
            <span>{t('history.resetWindow')}</span>
            <span>{t('history.status')}</span>
          </div>
          {recent.map((point) => (
            <div className="history-table-row" role="row" key={point.timestamp}>
              <span>
                {new Date(point.timestamp).toLocaleString(locale, {
                  month: 'short',
                  day: 'numeric',
                  hour: 'numeric',
                  minute: '2-digit',
                })}
              </span>
              <strong>
                {point.estimatedWeeklyValueUsd === null
                  ? '—'
                  : currency.format(point.estimatedWeeklyValueUsd)}
              </strong>
              <span>
                {point.observedCostUsd === null ? '—' : currency.format(point.observedCostUsd)}
              </span>
              <span>
                {point.weeklyUsedPercent === null ? '—' : `${Math.round(point.weeklyUsedPercent)}%`}
              </span>
              <span>
                {point.resetReason
                  ? formatResetReason(locale, point.resetReason)
                  : t('history.weeklyWindow')}
              </span>
              <span className={`table-status ${point.isFinalized ? 'finalized' : 'settling'}`}>
                {point.isFinalized ? t('history.finalized') : t('status.settling')}
              </span>
            </div>
          ))}
        </div>
      </div>
      <div className="pre-nerftrack-note">
        <Icon name="info" size={18} />
        <span>
          <strong>{t('history.pendingTitle')}</strong> {t('history.pendingDescription')}
        </span>
      </div>
    </section>
  );
}
