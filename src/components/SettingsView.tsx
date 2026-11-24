import { useState } from 'react';
import type { AppSettings, CustomPriceOverride } from '../domain';
import { useI18n, type LocalePreference, type MessageKey } from '../i18n';
import { Icon } from './Icons';
import { ReleaseNotes } from './ReleaseNotes';

interface SettingsViewProps {
  settings: AppSettings;
  detectedModelIds: string[];
  onChange: (key: keyof AppSettings, value: number | boolean | string) => void;
  onCustomPricingChange: (prices: CustomPriceOverride[]) => Promise<void>;
  onResetAllData: () => Promise<void>;
  onRestoreLastCheckpoint: () => Promise<void>;
  onImportAllData: () => Promise<void>;
  onOpenStarterPage: () => void;
}

const advancedRows: Array<{
  key: keyof AppSettings;
  labelKey: MessageKey;
  descriptionKey: MessageKey;
  type: 'number' | 'toggle';
  min?: number;
  max?: number;
  step?: number;
  suffixKey?: MessageKey;
}> = [
  {
    key: 'reconciliationIntervalHours',
    labelKey: 'settings.reconciliationInterval',
    descriptionKey: 'settings.reconciliationDescription',
    type: 'number',
    min: 1,
    max: 24,
    suffixKey: 'settings.hours',
  },
  {
    key: 'monitoringGapMinutes',
    labelKey: 'settings.monitoringGap',
    descriptionKey: 'settings.monitoringGapDescription',
    type: 'number',
    min: 1,
    max: 30,
    suffixKey: 'settings.minutes',
  },
  {
    key: 'reducedMotion',
    labelKey: 'settings.reducedMotion',
    descriptionKey: 'settings.reducedMotionDescription',
    type: 'toggle',
  },
];

const createPricingOverride = (modelId = ''): CustomPriceOverride => ({
  modelId,
  alias: null,
  inputUsdPerMillion: 0,
  cachedInputUsdPerMillion: 0,
  outputUsdPerMillion: 0,
});

export function SettingsView({
  settings,
  detectedModelIds,
  onChange,
  onCustomPricingChange,
  onResetAllData,
  onRestoreLastCheckpoint,
  onImportAllData,
  onOpenStarterPage,
}: SettingsViewProps) {
  const { t } = useI18n();
  const [dataAction, setDataAction] = useState<
    'idle' | 'resetting' | 'restoring-checkpoint' | 'importing-all'
  >('idle');
  const [confirmReset, setConfirmReset] = useState(false);
  const [dataMessage, setDataMessage] = useState<{
    key: MessageKey;
    values?: Record<string, string | number>;
  } | null>(null);
  const [pricingDraft, setPricingDraft] = useState<CustomPriceOverride[]>(settings.customPricing);
  const [pricingError, setPricingError] = useState<MessageKey | null>(null);
  const [pricingSaving, setPricingSaving] = useState(false);
  const draftModelIds = new Set(pricingDraft.map((price) => price.modelId.trim().toLowerCase()));
  const availableDetectedModelIds = detectedModelIds.filter(
    (modelId) => modelId.trim() && !draftModelIds.has(modelId.trim().toLowerCase()),
  );

  const autofillDetectedModels = () => {
    setPricingDraft((current) => {
      const existing = new Set(current.map((price) => price.modelId.trim().toLowerCase()));
      const additions = detectedModelIds
        .filter((modelId) => modelId.trim() && !existing.has(modelId.trim().toLowerCase()))
        .map((modelId) => createPricingOverride(modelId));
      return additions.length ? [...current, ...additions] : current;
    });
    setPricingError(null);
  };

  const savePricing = async () => {
    for (const price of pricingDraft) {
      if (!price.modelId.trim()) return setPricingError('settings.modelRequired');
      if (
        [price.inputUsdPerMillion, price.cachedInputUsdPerMillion, price.outputUsdPerMillion].some(
          (value) => !Number.isFinite(value) || value < 0,
        )
      ) {
        return setPricingError('settings.invalidPrices');
      }
    }
    setPricingSaving(true);
    setPricingError(null);
    try {
      await onCustomPricingChange(pricingDraft);
    } catch {
      setPricingError('settings.savePricingFailed');
    } finally {
      setPricingSaving(false);
    }
  };

  const runDataAction = async (action: 'resetting' | 'restoring-checkpoint' | 'importing-all') => {
    if (dataAction !== 'idle') return;
    setDataAction(action);
    setDataMessage(null);
    try {
      if (action === 'resetting') await onResetAllData();
      else if (action === 'restoring-checkpoint') await onRestoreLastCheckpoint();
      else await onImportAllData();
      setDataMessage(
        action === 'resetting'
          ? { key: 'settings.resetSuccess' }
          : action === 'restoring-checkpoint'
            ? { key: 'settings.restoreSuccess' }
            : { key: 'settings.importSuccess' },
      );
    } catch (error) {
      setDataMessage(
        action === 'resetting'
          ? { key: 'settings.resetFailed', values: { error: String(error) } }
          : action === 'restoring-checkpoint'
            ? { key: 'settings.restoreFailed', values: { error: String(error) } }
            : { key: 'settings.importFailed', values: { error: String(error) } },
      );
    } finally {
      setDataAction('idle');
    }
  };

  return (
    <section className="page-shell settings-page">
      <header className="page-heading">
        <h1>{t('settings.title')}</h1>
        <p>{t('settings.description')}</p>
      </header>
      <ReleaseNotes />
      <div className="panel defaults-panel">
        <div>
          <span className="settings-kicker">{t('settings.language')}</span>
          <strong>{t(`settings.language.${settings.locale}`)}</strong>
        </div>
        <label className="select-wrap">
          <span className="sr-only">{t('settings.language')}</span>
          <select
            aria-label={t('settings.language')}
            value={settings.locale}
            onChange={(event) => onChange('locale', event.target.value as LocalePreference)}
          >
            {(['system', 'en-US', 'zh-CN', 'zh-TW'] as const).map((locale) => (
              <option key={locale} value={locale}>
                {t(`settings.language.${locale}`)}
              </option>
            ))}
          </select>
          <Icon name="chevron" size={16} />
        </label>
      </div>
      <div className="settings-layout">
        <div className="panel settings-panel">
          <div className="panel-heading">
            <Icon name="settings" size={23} />
            <h2>{t('settings.advanced')}</h2>
          </div>
          {advancedRows.map((row) => (
            <div className="advanced-row" key={row.key}>
              <div className="advanced-copy">
                <strong>{t(row.labelKey)}</strong>
                <span>{t(row.descriptionKey)}</span>
              </div>
              {row.type === 'toggle' ? (
                <button
                  className={`toggle ${settings[row.key] ? 'on' : ''}`}
                  role="switch"
                  aria-checked={Boolean(settings[row.key])}
                  aria-label={t(row.labelKey)}
                  onClick={() => onChange(row.key, !settings[row.key])}
                >
                  <span />
                </button>
              ) : (
                <label className="number-input">
                  <span className="sr-only">{t(row.labelKey)}</span>
                  <input
                    type="number"
                    min={row.min}
                    max={row.max}
                    step={row.step ?? 1}
                    value={settings[row.key] as number}
                    onChange={(event) => onChange(row.key, Number(event.target.value))}
                  />
                  <em>{row.suffixKey ? ` ${t(row.suffixKey)}` : ''}</em>
                </label>
              )}
            </div>
          ))}
        </div>
        <div className="panel privacy-settings-panel">
          <div className="privacy-large-icon">
            <Icon name="lock" size={29} />
          </div>
          <h2>{t('settings.privacyFirst')}</h2>
          <p>{t('settings.privacyDescription')}</p>
          <div className="privacy-check">
            <Icon name="check" size={17} />
            {t('settings.localStorage')}
          </div>
          <div className="privacy-check">
            <Icon name="check" size={17} />
            {t('settings.releaseChecks')}
          </div>
          <div className="privacy-check">
            <Icon name="check" size={17} />
            {t('settings.localPrices')}
          </div>
        </div>
      </div>
      <section className="panel custom-pricing-panel" aria-labelledby="custom-pricing-heading">
        <div className="panel-heading">
          <Icon name="settings" size={23} />
          <h2 id="custom-pricing-heading">{t('settings.customPricing')}</h2>
        </div>
        <p>{t('settings.pricingDescription')}</p>
        <p className="settings-note">{t('settings.pricingNote')}</p>
        {detectedModelIds.length > 0 && (
          <div className="detected-models-callout">
            <div className="detected-models-copy">
              <Icon name={availableDetectedModelIds.length > 0 ? 'activity' : 'check'} size={18} />
              <div>
                <strong>
                  {availableDetectedModelIds.length > 0
                    ? t(
                        availableDetectedModelIds.length === 1
                          ? 'settings.unpricedModel'
                          : 'settings.unpricedModels',
                        { count: availableDetectedModelIds.length },
                      )
                    : t('settings.allDetectedDrafted')}
                </strong>
                <span>
                  {availableDetectedModelIds.length > 0
                    ? availableDetectedModelIds.join(' · ')
                    : t('settings.reviewPrices')}
                </span>
              </div>
            </div>
            {availableDetectedModelIds.length > 0 && (
              <button
                type="button"
                className="detected-models-button"
                onClick={autofillDetectedModels}
              >
                {t(
                  availableDetectedModelIds.length === 1
                    ? 'settings.autofillModel'
                    : 'settings.autofillModels',
                )}
              </button>
            )}
          </div>
        )}
        <div className="custom-price-grid" role="group" aria-label={t('settings.pricingGroup')}>
          {pricingDraft.map((price, index) => (
            <div className="custom-price-row" key={index}>
              <label>
                {t('settings.modelId')}
                <input
                  aria-label={`${t('settings.modelId')} ${index + 1}`}
                  value={price.modelId}
                  onChange={(event) =>
                    setPricingDraft((current) =>
                      current.map((item, itemIndex) =>
                        itemIndex === index ? { ...item, modelId: event.target.value } : item,
                      ),
                    )
                  }
                />
              </label>
              <label>
                {t('settings.alias')}
                <input
                  aria-label={`${t('settings.alias')} ${index + 1}`}
                  value={price.alias ?? ''}
                  onChange={(event) =>
                    setPricingDraft((current) =>
                      current.map((item, itemIndex) =>
                        itemIndex === index ? { ...item, alias: event.target.value || null } : item,
                      ),
                    )
                  }
                />
              </label>
              {(
                ['inputUsdPerMillion', 'cachedInputUsdPerMillion', 'outputUsdPerMillion'] as const
              ).map((field) => (
                <label key={field}>
                  {field === 'inputUsdPerMillion'
                    ? t('settings.input')
                    : field === 'cachedInputUsdPerMillion'
                      ? t('settings.cachedInput')
                      : t('settings.output')}
                  <input
                    aria-label={`${
                      field === 'inputUsdPerMillion'
                        ? t('settings.input')
                        : field === 'cachedInputUsdPerMillion'
                          ? t('settings.cachedInput')
                          : t('settings.output')
                    } ${index + 1}`}
                    type="number"
                    min="0"
                    step="any"
                    value={price[field]}
                    onChange={(event) =>
                      setPricingDraft((current) =>
                        current.map((item, itemIndex) =>
                          itemIndex === index
                            ? { ...item, [field]: Number(event.target.value) }
                            : item,
                        ),
                      )
                    }
                  />
                </label>
              ))}
              <button
                type="button"
                className="text-button"
                onClick={() =>
                  setPricingDraft((current) =>
                    current.filter((_, itemIndex) => itemIndex !== index),
                  )
                }
              >
                {t('settings.remove')}
              </button>
            </div>
          ))}
        </div>
        {pricingError && (
          <p className="settings-error" role="alert">
            {t(pricingError)}
          </p>
        )}
        <div className="custom-price-actions">
          <button
            type="button"
            onClick={() => setPricingDraft((current) => [...current, createPricingOverride()])}
          >
            {t('settings.addOverride')}
          </button>
          <button
            type="button"
            className="data-action-button"
            disabled={pricingSaving}
            onClick={() => void savePricing()}
          >
            {pricingSaving ? t('settings.saving') : t('settings.savePricing')}
          </button>
        </div>
      </section>
      <section className="panel starter-settings-panel" aria-labelledby="starter-settings-heading">
        <div>
          <span className="settings-kicker">{t('settings.community')}</span>
          <h2 id="starter-settings-heading">{t('settings.starterPage')}</h2>
          <p>{t('settings.starterDescription')}</p>
        </div>
        <button type="button" className="data-action-button" onClick={onOpenStarterPage}>
          <Icon name="refresh" size={15} />
          {t('settings.openStarter')}
        </button>
      </section>
      <section className="panel data-management-panel" aria-labelledby="data-management-heading">
        <div className="panel-heading">
          <Icon name="history" size={23} />
          <h2 id="data-management-heading">{t('settings.dataManagement')}</h2>
        </div>
        <p className="data-management-intro">{t('settings.dataIntro')}</p>
        <div className="data-action-grid">
          <div className="data-action-copy">
            <strong>{t('settings.resetAllData')}</strong>
            <span>{t('settings.resetDescription')}</span>
            <button
              className="data-action-button danger"
              disabled={dataAction !== 'idle'}
              onClick={() => {
                setDataMessage(null);
                setConfirmReset(true);
              }}
            >
              {t('settings.resetAllData')}
            </button>
          </div>
          <div className="data-action-copy">
            <div className="data-action-heading">
              <Icon name="history" size={17} />
              <strong>{t('settings.restoreCheckpoint')}</strong>
            </div>
            <span>{t('settings.restoreDescription')}</span>
            <button
              className="data-action-button"
              disabled={dataAction !== 'idle'}
              onClick={() => void runDataAction('restoring-checkpoint')}
            >
              <Icon name="refresh" size={15} />
              {dataAction === 'restoring-checkpoint'
                ? t('settings.restoring')
                : t('settings.restoreLast')}
            </button>
          </div>
          <div className="data-action-copy">
            <div className="data-action-heading">
              <Icon name="refresh" size={17} />
              <strong>{t('settings.importAll')}</strong>
            </div>
            <span>{t('settings.importDescription')}</span>
            <button
              className="data-action-button secondary"
              disabled={dataAction !== 'idle'}
              onClick={() => void runDataAction('importing-all')}
            >
              <Icon name="refresh" size={15} />
              {dataAction === 'importing-all' ? t('settings.importing') : t('settings.importAll')}
            </button>
          </div>
        </div>
        {confirmReset && dataAction === 'idle' && (
          <div
            className="data-confirmation"
            role="alertdialog"
            aria-labelledby="reset-confirm-heading"
          >
            <div>
              <strong id="reset-confirm-heading">{t('settings.confirmResetTitle')}</strong>
              <span>{t('settings.confirmResetDescription')}</span>
            </div>
            <div className="data-confirmation-actions">
              <button className="data-action-button quiet" onClick={() => setConfirmReset(false)}>
                {t('settings.cancel')}
              </button>
              <button
                className="data-action-button danger"
                onClick={() => {
                  setConfirmReset(false);
                  void runDataAction('resetting');
                }}
              >
                {t('settings.confirmReset')}
              </button>
            </div>
          </div>
        )}
        {dataMessage && (
          <p className="data-action-message" role="status">
            {t(dataMessage.key, dataMessage.values)}
          </p>
        )}
      </section>
    </section>
  );
}
