import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import App, { HomeView } from './App';
import type { HistoryPoint, HistoryResponse } from './domain';
import { demoQuote, demoStatus } from './lib/fixtures';

function historyPoint(overrides: Partial<HistoryPoint> = {}): HistoryPoint {
  return {
    timestamp: 0,
    estimatedWeeklyValueUsd: 100,
    rawEstimatedWeeklyValueUsd: 999,
    observedCostUsd: 1,
    weeklyUsedPercent: 20,
    resetAt: null,
    resetReason: null,
    isFinalized: true,
    isHeartbeat: false,
    epoch: 1,
    confidence: 'high',
    percentageCoverage: 20,
    ...overrides,
  };
}

function customHistory(points: HistoryPoint[], baseline = 10): HistoryResponse {
  return {
    points,
    statistics: {
      range: '1D',
      baselineEstimatedWeeklyValueUsd: baseline,
      baselineTimestamp: points[0]?.timestamp ?? null,
      currentEstimatedWeeklyValueUsd: points.at(-1)?.estimatedWeeklyValueUsd ?? null,
      deltaValueUsd: null,
      deltaPercent: null,
      pointCount: points.length,
      partial: false,
    },
    bucket: '5m',
  };
}

function renderHomeWithHistory(history: HistoryResponse) {
  render(
    <HomeView
      status={demoStatus}
      quote={demoQuote}
      history={history}
      annotations={[]}
      range="1D"
      reducedMotion={false}
      isRefreshing={false}
      onRefresh={vi.fn()}
      onRangeChange={vi.fn()}
      onResetAnnotations={vi.fn()}
    />,
  );
  const chart = screen.getByRole('img', {
    name: /Estimated weekly API-equivalent value/,
  });
  vi.spyOn(chart, 'getBoundingClientRect').mockReturnValue({
    left: 0,
    top: 0,
    width: 1000,
    height: 308,
    right: 1000,
    bottom: 308,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  });
  return chart;
}

describe('NerfTrack app shell', () => {
  it('renders the dashboard reference surface with a non-zero quote', async () => {
    render(<App />);
    expect(await screen.findByText('Codex Weekly API-equivalent Estimator')).toBeInTheDocument();
    expect(screen.getAllByText('≈$371').length).toBeGreaterThan(0);
    expect(screen.getByText('Weekly Used')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Refresh data' })).toBeInTheDocument();
    expect(screen.getByText(/Live ·/)).toBeInTheDocument();
    expect(await screen.findByText('CLI Mode · 846 usage events observed')).toBeInTheDocument();
  });

  it('keeps refresh beside the ranges and share as the final hero action', async () => {
    render(<App />);
    await screen.findByText('Codex Weekly API-equivalent Estimator');

    const rangeTabs = screen.getByRole('tablist', { name: 'History range' });
    const controls = rangeTabs.parentElement;
    expect(controls).not.toBeNull();
    expect(
      Array.from(controls!.querySelectorAll('button')).map((button) => button.textContent),
    ).toEqual(['1D', '1W', '1M', '3M', '6M', '', 'Share your graph']);
  });

  it('switches to setup and changes a monitoring control', async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole('button', { name: 'Setup' }));
    expect(screen.getByText('Set up NerfTrack')).toBeInTheDocument();
    expect(screen.getByText('Location hidden')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Retry detection' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Start monitoring' })).not.toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Reset saved selections' }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Need help/i })).not.toBeInTheDocument();
    const refreshSelect = screen.getByLabelText('Refresh interval');
    await user.selectOptions(refreshSelect, '20');
    expect(refreshSelect).toHaveValue('20');
  });

  it('switches the interface language from settings', async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    await user.selectOptions(screen.getByLabelText('Language'), 'zh-CN');

    expect(screen.getByRole('heading', { name: '设置' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '首页' })).toBeInTheDocument();
    expect(screen.getByLabelText('语言')).toHaveValue('zh-CN');
    expect(screen.getByRole('heading', { name: '高级监控' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '自定义 API 价格' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '隐私优先' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '添加覆盖价格' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '重置所有数据' })).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: '首页' }));
    expect(screen.getByRole('heading', { name: 'Codex 每周 API 等值估算' })).toBeInTheDocument();
    expect(screen.getByText('本周已使用')).toBeInTheDocument();
    expect(screen.getByText('稳定的每周 API 等值')).toBeInTheDocument();
    expect(screen.getByText('已观测 Token 成本')).toBeInTheDocument();
    expect(screen.getByRole('img', { name: /每周 API 等值估算历史图表/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '分享图表' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: '设置向导' }));
    expect(screen.getByRole('heading', { name: '设置 NerfTrack' })).toBeInTheDocument();
    expect(screen.getByText('位置已隐藏')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '重试检测' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '仅限本地' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: '诊断' }));
    expect(screen.getByRole('heading', { name: '诊断' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '质量原因' })).toBeInTheDocument();
    expect(screen.getByText('已观测事件')).toBeInTheDocument();
    expect(screen.getByText('模型缺少 API 价格')).toBeInTheDocument();
    expect(screen.getByText('报告的重置时间已变化')).toBeInTheDocument();
    expect(screen.getByText('正在等待成对的正增量')).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '已观测模型' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: '历史记录' }));
    expect(screen.getByRole('heading', { name: '历史记录' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '最近观测' })).toBeInTheDocument();
    expect(screen.getByText('当前')).toBeInTheDocument();
    expect(screen.getByText('日期')).toBeInTheDocument();
    expect(screen.getByText('状态')).toBeInTheDocument();
    expect(screen.getByText('记录方式')).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: '设置' }));
    await user.click(screen.getByRole('button', { name: '再次打开引导页' }));
    expect(screen.getByRole('heading', { name: '帮助 NerfTrack 持续发展。' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '不加星标，继续' })).toBeInTheDocument();
  });

  it('uses singular English observation copy for one valid observation', () => {
    render(
      <HomeView
        status={demoStatus}
        quote={{ ...demoQuote, confidence: 'low', validObservationCount: 1 }}
        history={customHistory([historyPoint()])}
        annotations={[]}
        range="1D"
        reducedMotion={false}
        isRefreshing={false}
        onRefresh={vi.fn()}
        onRangeChange={vi.fn()}
        onResetAnnotations={vi.fn()}
      />,
    );

    expect(screen.getByText('1 valid observation')).toBeInTheDocument();
    expect(screen.getByText(/from 1 valid observation and/)).toBeInTheDocument();
  });

  it('follows the system language while the preference is system', async () => {
    const languages = vi.spyOn(window.navigator, 'languages', 'get').mockReturnValue(['zh-TW']);

    render(<App />);

    expect(await screen.findByRole('button', { name: '首頁' })).toBeInTheDocument();
    languages.mockRestore();
  });

  it('renders the main surfaces in traditional Chinese', async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    await user.selectOptions(screen.getByLabelText('Language'), 'zh-TW');
    await user.click(screen.getByRole('button', { name: '首頁' }));

    expect(screen.getByRole('heading', { name: 'Codex 每週 API 等值估算' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '重新整理資料' })).toBeInTheDocument();
  });

  it('localizes routine reset reasons in history', async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    await user.selectOptions(screen.getByLabelText('Language'), 'zh-CN');
    await user.click(screen.getByRole('button', { name: '历史记录' }));

    expect(screen.getAllByText('计划重置').length).toBeGreaterThan(0);
  });

  it('gives the reduced-motion switch a localized accessible name', async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    await user.selectOptions(screen.getByLabelText('Language'), 'zh-CN');

    expect(screen.getByRole('switch', { name: '减少动态效果' })).toBeInTheDocument();
  });

  it('retranslates a visible settings validation message after switching language', async () => {
    const user = userEvent.setup();
    render(<App />);

    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    await user.click(screen.getByRole('button', { name: 'Add override' }));
    await user.click(screen.getByRole('button', { name: 'Save pricing' }));
    expect(screen.getByRole('alert')).toHaveTextContent('Each override needs a model ID.');

    await user.selectOptions(screen.getByLabelText('Language'), 'zh-CN');
    expect(screen.getByRole('alert')).toHaveTextContent('每条覆盖价格都需要模型 ID。');
  });

  it('shows an in-app confirmation before resetting local data', async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    await user.click(screen.getByRole('button', { name: 'Reset all data' }));

    expect(screen.getByRole('alertdialog')).toHaveTextContent('Reset all local data?');
    expect(screen.getByRole('button', { name: 'Confirm reset' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
  });

  it('keeps the GitHub update control and starter page accessible from settings', async () => {
    const user = userEvent.setup();
    render(<App />);

    expect(await screen.findByRole('button', { name: 'Up to date' })).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    await user.click(screen.getByRole('button', { name: 'Open starter page again' }));

    expect(screen.getByRole('heading', { name: 'Help NerfTrack keep going.' })).toBeInTheDocument();
    expect(screen.getByText('Let the resets continue')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Star NerfTrack on GitHub/ })).not.toBeDisabled();
    expect(screen.getByRole('button', { name: 'Continue without starring' })).toBeInTheDocument();
    expect(screen.queryByText(/follow on X/i)).not.toBeInTheDocument();
  });

  it('offers fast checkpoint restore and a separate full log import', async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole('button', { name: 'Settings' }));

    expect(screen.getByRole('button', { name: 'Restore last checkpoint' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Import all data' })).toBeInTheDocument();
    expect(screen.getByText(/fastest recovery option/i)).toBeInTheDocument();
    expect(screen.getByText(/re-read every available Codex log/i)).toBeInTheDocument();
  });

  it('edits and validates a local custom pricing override', async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole('button', { name: 'Settings' }));
    await user.click(screen.getByRole('button', { name: 'Add override' }));
    expect(screen.getByLabelText('Model ID 1')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Save pricing' }));
    expect(screen.getByRole('alert')).toHaveTextContent('Each override needs a model ID.');
    await user.type(screen.getByLabelText('Model ID 1'), 'local-codex');
    await user.click(screen.getByRole('button', { name: 'Save pricing' }));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('autofills custom pricing drafts from detected unpriced models', async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole('button', { name: 'Settings' }));

    expect(screen.getByText('local-codex-preview')).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Autofill detected model' }));

    expect(screen.getByLabelText('Model ID 1')).toHaveValue('local-codex-preview');
    expect(screen.getByText('All detected models are in this draft')).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Autofill detected model' }),
    ).not.toBeInTheDocument();
  });

  it('navigates to diagnostics without leaking sensitive fields', async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.click(await screen.findByRole('button', { name: 'Diagnostics' }));
    expect(screen.getByRole('heading', { name: 'Diagnostics' })).toBeInTheDocument();
    expect(screen.getByText(/Prompts, account identifiers/)).toBeInTheDocument();
  });

  it('shows the dollar and percentage difference across a held drag', async () => {
    render(<App />);
    const chart = await screen.findByRole('img', {
      name: /Estimated weekly API-equivalent value/,
    });
    vi.spyOn(chart, 'getBoundingClientRect').mockReturnValue({
      left: 0,
      top: 0,
      width: 1000,
      height: 308,
      right: 1000,
      bottom: 308,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    });

    fireEvent(chart, new MouseEvent('pointerdown', { bubbles: true, clientX: 400 }));
    fireEvent(chart, new MouseEvent('pointermove', { bubbles: true, clientX: 900 }));

    expect(screen.getByText('Selected range').parentElement).toHaveTextContent(
      /[+−]\$\d+\.\d{2} \([+−]\d+\.\d{2}%\)/,
    );
  });

  it('opens the Share Your Graph discussion from the home graph', async () => {
    const user = userEvent.setup();
    const onShareGraph = vi.fn().mockResolvedValue(undefined);
    render(
      <HomeView
        status={demoStatus}
        quote={demoQuote}
        history={customHistory([historyPoint()])}
        annotations={[]}
        range="1D"
        reducedMotion={false}
        isRefreshing={false}
        onRefresh={vi.fn()}
        onRangeChange={vi.fn()}
        onResetAnnotations={vi.fn()}
        onShareGraph={onShareGraph}
      />,
    );

    await user.click(screen.getByRole('button', { name: 'Share your graph' }));
    expect(onShareGraph).toHaveBeenCalledOnce();
  });

  it('shows a same-window calibration difference without neutral styling', () => {
    const chart = renderHomeWithHistory(
      customHistory([
        historyPoint({ estimatedWeeklyValueUsd: 94.35, percentageCoverage: 9 }),
        historyPoint({
          timestamp: 3_600_000,
          estimatedWeeklyValueUsd: 158.04,
          percentageCoverage: 53,
        }),
      ]),
    );

    fireEvent(chart, new MouseEvent('pointerdown', { bubbles: true, clientX: 0 }));
    fireEvent(chart, new MouseEvent('pointermove', { bubbles: true, clientX: 1000 }));

    expect(screen.getByText('Selected range').parentElement).toHaveTextContent(
      /[+−]\$\d+\.\d{2} \([+−]\d+\.\d{2}%\)/,
    );
    expect(screen.queryByText(/Comparison unavailable/)).not.toBeInTheDocument();
    expect(chart.closest('.usage-chart')).toHaveClass('chart-positive');
    expect(screen.getByText('≈$158')).toBeInTheDocument();
  });

  it('shows a difference for an immature cross-window anchor', () => {
    const chart = renderHomeWithHistory(
      customHistory([
        historyPoint({
          estimatedWeeklyValueUsd: 72.62,
          confidence: 'medium',
          percentageCoverage: 8,
        }),
        historyPoint({
          timestamp: 3_600_000,
          epoch: 2,
          estimatedWeeklyValueUsd: 160.84,
          percentageCoverage: 51,
        }),
      ]),
    );

    fireEvent(chart, new MouseEvent('pointerdown', { bubbles: true, clientX: 0 }));
    fireEvent(chart, new MouseEvent('pointermove', { bubbles: true, clientX: 1000 }));

    expect(screen.getByText('Selected range').parentElement).toHaveTextContent(
      /[+−]\$\d+\.\d{2} \([+−]\d+\.\d{2}%\)/,
    );
    expect(screen.queryByText(/Comparison unavailable/)).not.toBeInTheDocument();
    expect(chart.closest('.usage-chart')).toHaveClass('chart-positive');
  });

  it('shows a difference when hovering without an anchor', () => {
    const chart = renderHomeWithHistory(
      customHistory([
        historyPoint({ estimatedWeeklyValueUsd: 100 }),
        historyPoint({
          timestamp: 3_600_000,
          epoch: 2,
          estimatedWeeklyValueUsd: 200,
        }),
      ]),
    );
    const hoverEvent = new MouseEvent('pointermove', { bubbles: true, clientX: 1000 });
    Object.defineProperty(hoverEvent, 'pointerType', { value: 'mouse' });
    fireEvent(chart, hoverEvent);

    expect(screen.queryByText('Selected range')).not.toBeInTheDocument();
    expect(screen.getByText(/\(\+1900\.00%\)/)).toBeInTheDocument();
    expect(screen.queryByText(/Comparison unavailable/)).not.toBeInTheDocument();
  });

  it('switches cached ranges without remounting the chart or keeping a weekly label', async () => {
    const user = userEvent.setup();
    render(<App />);
    const chart = await screen.findByRole('img', {
      name: /Estimated weekly API-equivalent value/,
    });

    await user.click(screen.getByRole('tab', { name: '1M' }));

    expect(screen.getByText(/^Since /)).toBeInTheDocument();
    expect(screen.getByRole('img', { name: /Estimated weekly API-equivalent value/ })).toBe(chart);
  });
});
