import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import App from './App';
import { demoSettings, demoQuote, demoStatus, getDemoHistory } from './lib/fixtures';
import type { AppSettings, Range } from './domain';
import { CURRENT_APP_VERSION } from './lib/updater';

const controls = vi.hoisted(() => ({
  history: vi.fn(),
  status: vi.fn(),
  settings: vi.fn(),
  saveSettings: vi.fn(),
  desktop: false,
}));
vi.mock('./lib/commands', async (original) => ({
  ...(await original<typeof import('./lib/commands')>()),
  getSettings: controls.settings,
  updateSettings: controls.saveSettings,
  getHistory: controls.history,
  getCurrentStatus: controls.status,
  getCurrentQuote: async () => demoQuote,
}));

vi.mock('./lib/tauri', async (original) => ({
  ...(await original<typeof import('./lib/tauri')>()),
  isTauri: () => controls.desktop,
}));

describe('calculation release regressions', () => {
  it('allows a user to type a model ID without losing focus', async () => {
    controls.settings.mockResolvedValue({ ...demoSettings, starterPageSeen: true });
    controls.status.mockResolvedValue(demoStatus);
    controls.history.mockImplementation(async (range: Range) => getDemoHistory(range));
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole('tablist', { name: 'History range' });
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await user.click(screen.getByRole('button', { name: 'Add override' }));
    await user.type(screen.getByRole('textbox', { name: 'Model ID 1' }), 'example');
    expect(screen.getByRole('textbox', { name: 'Model ID 1' })).toHaveValue('example');
  });

  it('fetches the newly selected chart range after an overlapping refresh', async () => {
    controls.settings.mockResolvedValue({ ...demoSettings, starterPageSeen: true });
    controls.status.mockResolvedValue(demoStatus);
    controls.history.mockImplementation(async (range: Range) => getDemoHistory(range));
    render(<App />);
    const tabs = await screen.findByRole('tablist', { name: 'History range' });
    controls.history.mockClear();
    controls.status.mockClear();
    let finishRefresh!: (status: typeof demoStatus) => void;
    controls.status.mockReturnValueOnce(
      new Promise((resolve) => {
        finishRefresh = resolve;
      }),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Refresh data' }));
    await waitFor(() => expect(controls.status).toHaveBeenCalledTimes(1));
    fireEvent.click(within(tabs).getByRole('tab', { name: '1D' }));
    await act(async () => finishRefresh(demoStatus));
    expect(controls.history).toHaveBeenCalledWith('1D');
  });
  it('shows release notes once per app version and keeps them available in Settings', async () => {
    controls.desktop = true;
    try {
      let saved: AppSettings = {
        ...demoSettings,
        starterPageSeen: true,
        lastSeenReleaseNotesVersion: '1.1.7',
      };
      controls.settings.mockImplementation(async () => saved);
      controls.saveSettings.mockImplementation(async (settings: AppSettings) => {
        saved = settings;
        return saved;
      });
      controls.status.mockResolvedValue(demoStatus);
      controls.history.mockImplementation(async (range: Range) => getDemoHistory(range));
      const user = userEvent.setup();
      const title = `What’s new in v${CURRENT_APP_VERSION}`;
      const first = render(<App />);
      const notice = await screen.findByRole('complementary', { name: title });
      await user.click(within(notice).getByRole('button', { name: 'Dismiss' }));
      await waitFor(() => expect(notice).not.toBeInTheDocument());
      expect(saved.lastSeenReleaseNotesVersion).toBe(CURRENT_APP_VERSION);
      first.unmount();

      render(<App />);
      await screen.findByRole('tablist', { name: 'History range' });
      expect(screen.queryByRole('complementary', { name: title })).not.toBeInTheDocument();
      await user.click(screen.getByRole('button', { name: 'Settings' }));
      await user.click(screen.getByText(title));
      expect(screen.getByText(/Release notes are available in Settings/)).toBeVisible();
    } finally {
      controls.desktop = false;
    }
  });
});
