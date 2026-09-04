import type { AppSettings, AppStatus } from '../domain';
import { useI18n, type MessageKey } from '../i18n';
import { Icon, type IconName } from './Icons';

interface SetupViewProps {
  status: AppStatus;
  settings: AppSettings;
  onChooseHome: () => void;
  onChooseExecutable: () => void;
  onRetry: () => void;
  onSettingChange: (key: keyof AppSettings, value: number | boolean | string) => void;
}

const discoveryCards: Array<{
  key: 'codexHome' | 'codexExecutable' | 'appServer';
  titleKey: MessageKey;
  icon: IconName;
  actionKey?: MessageKey;
}> = [
  {
    key: 'codexHome',
    titleKey: 'setup.dataFolder',
    icon: 'folder',
    actionKey: 'setup.chooseFolder',
  },
  {
    key: 'codexExecutable',
    titleKey: 'setup.executable',
    icon: 'terminal',
    actionKey: 'setup.chooseExecutable',
  },
  { key: 'appServer', titleKey: 'setup.appServer', icon: 'server' },
];

const settingsRows: Array<{
  key: keyof AppSettings;
  icon: IconName;
  titleKey: MessageKey;
  descriptionKey: MessageKey;
  options: number[];
}> = [
  {
    key: 'refreshIntervalSeconds',
    icon: 'clock',
    titleKey: 'setup.refreshInterval',
    descriptionKey: 'setup.refreshDescription',
    options: [10, 20, 30],
  },
];

const discoveryStateKeys: Record<AppStatus['codexHome']['state'], MessageKey> = {
  auto_detected: 'discovery.auto_detected',
  selected: 'discovery.selected',
  missing: 'discovery.missing',
  unsupported: 'discovery.unsupported',
  redacted: 'discovery.redacted',
  not_required: 'discovery.not_required',
};

export function SetupView({
  status,
  settings,
  onChooseHome,
  onChooseExecutable,
  onRetry,
  onSettingChange,
}: SetupViewProps) {
  const { t } = useI18n();
  const guiMode = status.integrationMode === 'gui';

  return (
    <section className="setup-page page-shell">
      <header className="page-heading">
        <h1>{t('setup.title')}</h1>
        <p>{t(guiMode ? 'setup.description.desktop' : 'setup.description.cli')}</p>
      </header>
      <div className="discovery-grid">
        {discoveryCards.map((card) => {
          const discovery = status[card.key];
          const title =
            card.key === 'codexExecutable' && guiMode ? t('setup.cliExecutable') : t(card.titleKey);
          const label =
            card.key === 'codexExecutable' && guiMode
              ? t('setup.chooseCliExecutable')
              : card.actionKey
                ? t(card.actionKey)
                : undefined;
          const action =
            card.key === 'codexHome'
              ? onChooseHome
              : card.key === 'codexExecutable'
                ? onChooseExecutable
                : guiMode
                  ? onChooseExecutable
                  : onRetry;
          const isAlert = discovery.state === 'missing' || discovery.state === 'unsupported';
          const discoveryPath =
            discovery.redactedLocation === 'local path redacted'
              ? t('discovery.redacted')
              : (discovery.redactedLocation ??
                (discovery.state === 'not_required'
                  ? t('setup.notRequired')
                  : t('setup.notDiscovered')));
          return (
            <article className="discovery-card" key={card.key}>
              <div className="discovery-title-row">
                <div className="discovery-icon">
                  <Icon name={card.icon} size={28} />
                </div>
                <div>
                  <h2>{title}</h2>
                  <p className={`discovery-state ${isAlert ? 'missing' : ''}`}>
                    <Icon name={isAlert ? 'alert' : 'check'} size={17} />
                    {t(discoveryStateKeys[discovery.state])}
                  </p>
                </div>
              </div>
              <span className="discovery-path">{discoveryPath}</span>
              {label && (
                <button className="quiet-button discovery-action" onClick={action}>
                  {label}
                  <Icon name="chevron" size={15} />
                </button>
              )}
            </article>
          );
        })}
      </div>
      <div className="panel monitoring-panel">
        <div className="panel-heading">
          <Icon name="settings" size={23} />
          <h2>{t('setup.monitoringSettings')}</h2>
        </div>
        <div className="setting-rows">
          {settingsRows.map((row) => (
            <div className="setting-row" key={row.key}>
              <div className="setting-row-icon">
                <Icon name={row.icon} size={25} />
              </div>
              <div className="setting-copy">
                <strong>{t(row.titleKey)}</strong>
                <span>{t(row.descriptionKey)}</span>
              </div>
              <label className="select-wrap">
                <span className="sr-only">{t(row.titleKey)}</span>
                <select
                  value={settings[row.key] as number}
                  onChange={(event) => onSettingChange(row.key, Number(event.target.value))}
                >
                  {row.options.map((option) => (
                    <option key={option} value={option}>
                      {t('setup.seconds', { value: option })}
                    </option>
                  ))}
                </select>
                <Icon name="chevron" size={16} />
              </label>
            </div>
          ))}
        </div>
      </div>
      <div className="privacy-panel panel">
        <div className="privacy-icon">
          <Icon name="shield" size={36} strokeWidth={1.5} />
        </div>
        <div>
          <h2>{t('setup.localOnly')}</h2>
          <p>{t('setup.localDescription')}</p>
        </div>
        <span className="local-badge">
          <Icon name="lock" size={17} />
          {t('setup.localBadge')}
        </span>
      </div>
      <div className="setup-actions">
        <button className="secondary-button" onClick={onRetry}>
          <Icon name="refresh" size={21} />
          {t('setup.retry')}
        </button>
      </div>
    </section>
  );
}
