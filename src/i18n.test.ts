import { describe, expect, it } from 'vitest';
import { detectLocale, formatDiagnosticReason, formatResetReason, translate } from './i18n';

describe('internationalization', () => {
  it('maps supported system languages to the matching interface locale', () => {
    expect(detectLocale(['zh-Hant-HK'])).toBe('zh-TW');
    expect(detectLocale(['zh-CN'])).toBe('zh-CN');
    expect(detectLocale(['en-GB'])).toBe('en-US');
  });

  it('uses the first supported language and falls back to English', () => {
    expect(detectLocale(['fr-FR', 'zh-TW'])).toBe('zh-TW');
    expect(detectLocale(['fr-FR'])).toBe('en-US');
  });

  it('translates interface text and interpolates values', () => {
    expect(translate('zh-CN', 'nav.settings')).toBe('设置');
    expect(translate('zh-TW', 'common.version', { version: '1.1.4' })).toBe('版本 1.1.4');
    expect(translate('zh-CN', 'history.bucketRaw')).toBe('逐条记录');
  });

  it('translates routine reset reasons and preserves an English fallback', () => {
    expect(formatResetReason('zh-CN', 'scheduled_reset')).toBe('计划重置');
    expect(formatResetReason('zh-TW', 'Weekly window · reset changed')).toBe('重設已變更');
    expect(formatResetReason('zh-CN', 'uncertain_reset')).toBe('重置状态不确定');
    expect(formatResetReason('zh-TW', 'Weekly window · uncertain_reset')).toBe('重設狀態不確定');
    expect(formatResetReason('zh-CN', 'unknown_reset_reason')).toBe('Unknown reset reason');
  });

  it('translates common diagnostic reasons and preserves technical fallbacks', () => {
    expect(formatDiagnosticReason('zh-CN', 'monitoring gap')).toBe('监控中断');
    expect(formatDiagnosticReason('zh-TW', 'partial final line')).toBe('記錄仍在寫入');
    expect(
      formatDiagnosticReason(
        'zh-CN',
        'unknown API price for model local-codex; add a local custom price override',
      ),
    ).toBe('模型 local-codex 缺少 API 价格；请添加本地自定义价格。');
    expect(formatDiagnosticReason('zh-CN', 'non-finite token-derived API cost')).toBe(
      'non-finite token-derived API cost',
    );
  });
});
