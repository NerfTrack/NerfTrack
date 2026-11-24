import { useI18n, type MessageKey } from '../i18n';
import { CURRENT_APP_VERSION } from '../lib/updater';

const releaseChanges: MessageKey[] = [
  'releaseNotes.estimates',
  'releaseNotes.history',
  'releaseNotes.upgrade',
  'releaseNotes.updates',
];

export function ReleaseNotes({ onDismiss }: { onDismiss?: () => void }) {
  const { t } = useI18n();
  const heading = t('releaseNotes.title', { version: CURRENT_APP_VERSION });
  const changes = (
    <ul>
      {releaseChanges.map((key) => (
        <li key={key}>{t(key)}</li>
      ))}
    </ul>
  );
  return onDismiss ? (
    <aside className="panel release-notes" aria-label={heading}>
      <h2>{heading}</h2>
      {changes}
      <button type="button" className="data-action-button" onClick={onDismiss}>
        {t('releaseNotes.dismiss')}
      </button>
    </aside>
  ) : (
    <details className="panel release-notes">
      <summary>{heading}</summary>
      {changes}
    </details>
  );
}
