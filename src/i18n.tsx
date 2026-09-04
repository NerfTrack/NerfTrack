/* eslint-disable react-refresh/only-export-components */
import { createContext, useContext, useMemo, type ReactNode } from 'react';

export type Locale = 'en-US' | 'zh-CN' | 'zh-TW';
export type LocalePreference = 'system' | Locale;

const messages = {
  'nav.primary': ['Primary', '主导航', '主要導覽'],
  'nav.home': ['Home', '首页', '首頁'],
  'nav.setup': ['Setup', '设置向导', '設定精靈'],
  'nav.diagnostics': ['Diagnostics', '诊断', '診斷'],
  'nav.history': ['History', '历史记录', '歷史記錄'],
  'nav.settings': ['Settings', '设置', '設定'],
  'common.version': ['Version {version}', '版本 {version}', '版本 {version}'],
  'settings.title': ['Settings', '设置', '設定'],
  'settings.language': ['Language', '语言', '語言'],
  'settings.language.system': ['Use system language', '跟随系统语言', '跟隨系統語言'],
  'settings.language.en-US': ['English', 'English', 'English'],
  'settings.language.zh-CN': ['简体中文', '简体中文', '简体中文'],
  'settings.language.zh-TW': ['繁體中文', '繁體中文', '繁體中文'],
  'home.title': [
    'Codex Weekly API-equivalent Estimator',
    'Codex 每周 API 等值估算',
    'Codex 每週 API 等值估算',
  ],
  'home.refresh': ['Refresh data', '刷新数据', '重新整理資料'],
  'setup.title': ['Set up NerfTrack', '设置 NerfTrack', '設定 NerfTrack'],
  'diagnostics.title': ['Diagnostics', '诊断', '診斷'],
  'history.title': ['History', '历史记录', '歷史記錄'],
  'settings.advanced': ['Advanced monitoring', '高级监控', '進階監控'],
  'settings.customPricing': ['Custom API pricing', '自定义 API 价格', '自訂 API 價格'],
  'settings.resetAllData': ['Reset all data', '重置所有数据', '重設所有資料'],
  'home.shareGraph': ['Share your graph', '分享图表', '分享圖表'],
  'home.opening': ['Opening…', '正在打开…', '正在開啟…'],
  'home.weeklyUsed': ['Weekly Used', '本周已使用', '本週已使用'],
  'setup.retry': ['Retry detection', '重试检测', '重試偵測'],
  'diagnostics.qualityReasons': ['Quality reasons', '质量原因', '品質原因'],
  'history.recent': ['Recent observations', '最近观测', '最近觀測'],
  'update.checking': ['Checking for updates', '正在检查更新', '正在檢查更新'],
  'update.available': ['Update available', '有可用更新', '有可用更新'],
  'update.downloading': ['Downloading…', '正在下载…', '正在下載…'],
  'update.installing': ['Installing…', '正在安装…', '正在安裝…'],
  'update.upToDate': ['Up to date', '已是最新版本', '已是最新版本'],
  'update.failed': ['Update failed', '更新失败', '更新失敗'],
  'update.notConfigured': ['Updates not configured', '未配置更新', '未設定更新'],
  'update.check': ['Check for updates', '检查更新', '檢查更新'],
  'update.installedVersions': [
    'Installed v{current} · latest v{latest}',
    '已安装 v{current} · 最新 v{latest}',
    '已安裝 v{current} · 最新 v{latest}',
  ],
  'status.connected': ['Connected', '已连接', '已連線'],
  'status.detecting': ['Detecting', '正在检测', '正在偵測'],
  'status.settling': ['Settling', '正在稳定', '正在穩定'],
  'status.recalibrating': ['Updating local data', '正在更新本地数据', '正在更新本機資料'],
  'status.unsupported': ['Unsupported', '不受支持', '不支援'],
  'status.needs_setup': ['Needs setup', '需要设置', '需要設定'],
  'status.error': ['Unavailable', '不可用', '無法使用'],
  'status.cliMode': ['CLI Mode', 'CLI 模式', 'CLI 模式'],
  'status.desktopMode': ['Desktop Mode', '桌面模式', '桌面模式'],
  'status.localMode': ['Local Mode', '本地模式', '本機模式'],
  'status.waitingForUsage': [
    '{mode} · waiting for usage',
    '{mode} · 正在等待用量',
    '{mode} · 正在等待用量',
  ],
  'status.updatingDetail': [
    '{mode} · updating local data',
    '{mode} · 正在更新本地数据',
    '{mode} · 正在更新本機資料',
  ],
  'status.unableToRead': [
    '{mode} · unable to read local data',
    '{mode} · 无法读取本地数据',
    '{mode} · 無法讀取本機資料',
  ],
  'status.observedEvent': [
    '{mode} · 1 usage event observed',
    '{mode} · 已观测 1 个用量事件',
    '{mode} · 已觀測 1 個用量事件',
  ],
  'status.observedEvents': [
    '{mode} · {count} usage events observed',
    '{mode} · 已观测 {count} 个用量事件',
    '{mode} · 已觀測 {count} 個用量事件',
  ],
  'setup.description.desktop': [
    'Connect local Codex desktop data to estimate weekly API-equivalent value from tokens.',
    '连接本地 Codex 桌面数据，根据 Token 用量估算每周 API 等值。',
    '連接本機 Codex 桌面資料，根據 Token 用量估算每週 API 等值。',
  ],
  'setup.description.cli': [
    'Connect local Codex data from the desktop app or CLI to estimate weekly API-equivalent value from tokens.',
    '连接桌面应用或 CLI 的本地 Codex 数据，根据 Token 用量估算每周 API 等值。',
    '連接桌面應用程式或 CLI 的本機 Codex 資料，根據 Token 用量估算每週 API 等值。',
  ],
  'setup.dataFolder': ['Codex data folder', 'Codex 数据文件夹', 'Codex 資料夾'],
  'setup.chooseFolder': ['Choose folder', '选择文件夹', '選擇資料夾'],
  'setup.executable': ['Codex executable', 'Codex 可执行文件', 'Codex 執行檔'],
  'setup.chooseExecutable': ['Choose executable', '选择可执行文件', '選擇執行檔'],
  'setup.cliExecutable': ['Codex CLI executable', 'Codex CLI 可执行文件', 'Codex CLI 執行檔'],
  'setup.chooseCliExecutable': ['Choose CLI executable', '选择 CLI 可执行文件', '選擇 CLI 執行檔'],
  'setup.appServer': ['App Server (CLI only)', 'App Server（仅 CLI）', 'App Server（僅 CLI）'],
  'setup.notRequired': ['Not required in desktop mode', '桌面模式无需设置', '桌面模式不需設定'],
  'setup.notDiscovered': ['Not discovered yet', '尚未发现', '尚未找到'],
  'discovery.auto_detected': ['Auto-detected', '已自动检测', '已自動偵測'],
  'discovery.selected': ['Selected', '已选择', '已選擇'],
  'discovery.missing': ['Not found', '未找到', '找不到'],
  'discovery.unsupported': ['Unsupported', '不受支持', '不支援'],
  'discovery.redacted': ['Location hidden', '位置已隐藏', '位置已隱藏'],
  'discovery.not_required': ['Not required', '无需设置', '不需設定'],
  'setup.monitoringSettings': ['Monitoring settings', '监控设置', '監控設定'],
  'setup.refreshInterval': ['Refresh interval', '刷新间隔', '重新整理間隔'],
  'setup.refreshDescription': [
    'How often NerfTrack checks for new Codex usage.',
    'NerfTrack 检查新 Codex 用量的频率。',
    'NerfTrack 檢查新 Codex 用量的頻率。',
  ],
  'setup.seconds': ['{value} seconds', '{value} 秒', '{value} 秒'],
  'setup.localOnly': ['Local-only', '仅限本地', '僅限本機'],
  'setup.localDescription': [
    'All processing and data storage happen only on this machine. No data leaves your device.',
    '所有处理和数据存储都只在本机进行，不会有数据离开你的设备。',
    '所有處理和資料儲存都只在本機進行，不會有資料離開你的裝置。',
  ],
  'setup.localBadge': ['100% Local', '100% 本地', '100% 本機'],
  'settings.description': [
    'Local defaults for monitoring, privacy, and presentation.',
    '监控、隐私和显示方式的本地默认设置。',
    '監控、隱私和顯示方式的本機預設設定。',
  ],
  'settings.reconciliationInterval': ['Reconciliation interval', '重新扫描间隔', '重新掃描間隔'],
  'settings.reconciliationDescription': [
    'Re-scan known files and recover missed notifications.',
    '重新扫描已知文件并补回遗漏的通知。',
    '重新掃描已知檔案並補回遺漏的通知。',
  ],
  'settings.monitoringGap': ['Monitoring gap threshold', '监控中断阈值', '監控中斷門檻'],
  'settings.monitoringGapDescription': [
    'Record a collection interruption for diagnostics.',
    '记录采集中断，供诊断使用。',
    '記錄收集中斷，供診斷使用。',
  ],
  'settings.reducedMotion': ['Reduced motion', '减少动态效果', '減少動態效果'],
  'settings.reducedMotionDescription': [
    'Disable estimate-finalization animations and motion cues.',
    '禁用估算完成动画和动态提示。',
    '停用估算完成動畫和動態提示。',
  ],
  'settings.hours': ['h', '小时', '小時'],
  'settings.minutes': ['min', '分钟', '分鐘'],
  'settings.privacyFirst': ['Privacy first', '隐私优先', '隱私優先'],
  'settings.privacyDescription': [
    'NerfTrack runs locally. No prompts, code, raw account identifiers, or telemetry leave this device; only public pricing metadata is refreshed at launch.',
    'NerfTrack 在本地运行。提示词、代码、原始账户标识和遥测数据都不会离开此设备；启动时只会刷新公开的价格元数据。',
    'NerfTrack 在本機執行。提示詞、程式碼、原始帳戶識別資訊和遙測資料都不會離開此裝置；啟動時只會重新整理公開的價格中繼資料。',
  ],
  'settings.localStorage': ['Local-only storage', '仅本地存储', '僅本機儲存'],
  'settings.releaseChecks': [
    'GitHub Releases checks stay opt-in to configuration',
    'GitHub Releases 检查仅在配置后启用',
    'GitHub Releases 檢查僅在設定後啟用',
  ],
  'settings.localPrices': [
    'Token prices stay local',
    'Token 价格保留在本地',
    'Token 價格保留在本機',
  ],
  'settings.pricingDescription': [
    'Overrides are local only and take precedence over models.dev and NerfTrack’s embedded fallback rates. Use them for an unpriced model or a local alias; prices are USD per 1M tokens.',
    '覆盖价格仅保存在本地，并优先于 models.dev 和 NerfTrack 内置的备用价格。可用于尚未定价的模型或本地别名；价格单位为每百万 Token 的美元金额。',
    '覆寫價格僅儲存在本機，並優先於 models.dev 和 NerfTrack 內建的備用價格。可用於尚未定價的模型或本機別名；價格單位為每百萬 Token 的美元金額。',
  ],
  'settings.pricingNote': [
    'NerfTrack refreshes the public models.dev OpenAI catalog at each launch and caches the last valid response. If it is unavailable, embedded OpenAI fallback rates are used. Unknown models remain pending until a catalog price or override is available.',
    'NerfTrack 每次启动都会刷新公开的 models.dev OpenAI 目录，并缓存最后一次有效响应。如果目录不可用，则使用内置的 OpenAI 备用价格。未知模型会保持待定，直到目录价格或覆盖价格可用。',
    'NerfTrack 每次啟動都會重新整理公開的 models.dev OpenAI 目錄，並快取最後一次有效回應。如果目錄無法使用，則使用內建的 OpenAI 備用價格。未知模型會保持待定，直到目錄價格或覆寫價格可用。',
  ],
  'settings.unpricedModels': [
    '{count} unpriced models detected',
    '检测到 {count} 个未定价模型',
    '偵測到 {count} 個未定價模型',
  ],
  'settings.unpricedModel': [
    '1 unpriced model detected',
    '检测到 1 个未定价模型',
    '偵測到 1 個未定價模型',
  ],
  'settings.allDetectedDrafted': [
    'All detected models are in this draft',
    '所有检测到的模型都已加入草稿',
    '所有偵測到的模型都已加入草稿',
  ],
  'settings.reviewPrices': [
    'Review the prices below, then save when ready.',
    '检查下方价格，确认后保存。',
    '檢查下方價格，確認後儲存。',
  ],
  'settings.autofillModel': [
    'Autofill detected model',
    '自动填入检测到的模型',
    '自動填入偵測到的模型',
  ],
  'settings.autofillModels': [
    'Autofill detected models',
    '自动填入检测到的模型',
    '自動填入偵測到的模型',
  ],
  'settings.pricingGroup': [
    'Custom API pricing overrides',
    '自定义 API 覆盖价格',
    '自訂 API 覆寫價格',
  ],
  'settings.modelId': ['Model ID', '模型 ID', '模型 ID'],
  'settings.alias': ['Alias', '别名', '別名'],
  'settings.input': ['Input', '输入', '輸入'],
  'settings.cachedInput': ['Cached input', '缓存输入', '快取輸入'],
  'settings.output': ['Output', '输出', '輸出'],
  'settings.remove': ['Remove', '移除', '移除'],
  'settings.addOverride': ['Add override', '添加覆盖价格', '新增覆寫價格'],
  'settings.saving': ['Saving…', '正在保存…', '正在儲存…'],
  'settings.savePricing': ['Save pricing', '保存价格', '儲存價格'],
  'settings.modelRequired': [
    'Each override needs a model ID.',
    '每条覆盖价格都需要模型 ID。',
    '每筆覆寫價格都需要模型 ID。',
  ],
  'settings.invalidPrices': [
    'Prices must be finite, non-negative USD amounts.',
    '价格必须是有限且非负的美元金额。',
    '價格必須是有限且非負的美元金額。',
  ],
  'settings.savePricingFailed': [
    'Could not save local pricing overrides.',
    '无法保存本地覆盖价格。',
    '無法儲存本機覆寫價格。',
  ],
  'settings.community': ['Community', '社区', '社群'],
  'settings.starterPage': ['Starter page', '引导页', '引導頁'],
  'settings.starterDescription': [
    'Reopen the GitHub support page any time. NerfTrack will remember that you have seen it again after you continue.',
    '随时重新打开 GitHub 支持页面。继续后，NerfTrack 会记住你已再次查看。',
    '隨時重新開啟 GitHub 支援頁面。繼續後，NerfTrack 會記住你已再次查看。',
  ],
  'settings.openStarter': ['Open starter page again', '再次打开引导页', '再次開啟引導頁'],
  'settings.dataManagement': ['Data management', '数据管理', '資料管理'],
  'settings.dataIntro': [
    'Reset local history without touching Codex logs. A fast checkpoint is saved immediately before every reset, while a full import can rebuild the index from every readable log.',
    '重置本地历史记录而不改动 Codex 日志。每次重置前都会立即保存快速检查点；完整导入可从所有可读日志重建索引。',
    '重設本機歷史記錄而不更動 Codex 日誌。每次重設前都會立即儲存快速檢查點；完整匯入可從所有可讀日誌重建索引。',
  ],
  'settings.resetDescription': [
    'Clear imported usage, quota observations, graphs, diagnostics, and annotations. New Codex activity is monitored immediately after reset.',
    '清除已导入的用量、额度观测、图表、诊断和注释。重置后立即监控新的 Codex 活动。',
    '清除已匯入的用量、額度觀測、圖表、診斷和註解。重設後立即監控新的 Codex 活動。',
  ],
  'settings.restoreCheckpoint': [
    'Restore from last checkpoint',
    '从上一个检查点恢复',
    '從上一個檢查點還原',
  ],
  'settings.restoreDescription': [
    'Restore the graph captured immediately before the latest reset, then index only newer activity. This is the fastest recovery option.',
    '恢复最近一次重置前保存的图表，然后只索引更新的活动。这是最快的恢复方式。',
    '還原最近一次重設前儲存的圖表，然後只索引更新的活動。這是最快的還原方式。',
  ],
  'settings.restoring': ['Restoring checkpoint…', '正在恢复检查点…', '正在還原檢查點…'],
  'settings.restoreLast': ['Restore last checkpoint', '恢复上一个检查点', '還原上一個檢查點'],
  'settings.importAll': ['Import all data', '导入所有数据', '匯入所有資料'],
  'settings.importDescription': [
    'Clear the import index and re-read every available Codex log from the beginning. Best for a complete rebuild; may take longer.',
    '清除导入索引，并从头重新读取所有可用的 Codex 日志。适合完整重建，但可能需要更长时间。',
    '清除匯入索引，並從頭重新讀取所有可用的 Codex 日誌。適合完整重建，但可能需要較長時間。',
  ],
  'settings.importing': ['Importing all data…', '正在导入所有数据…', '正在匯入所有資料…'],
  'settings.confirmResetTitle': [
    'Reset all local data?',
    '重置所有本地数据？',
    '重設所有本機資料？',
  ],
  'settings.confirmResetDescription': [
    'This clears NerfTrack’s imported usage, quota observations, graph history, diagnostics, annotations, and scan checkpoints. Codex source logs are not deleted; monitoring resumes from the current end of those logs.',
    '这会清除 NerfTrack 已导入的用量、额度观测、图表历史、诊断、注释和扫描检查点。Codex 源日志不会被删除；监控会从这些日志当前的末尾继续。',
    '這會清除 NerfTrack 已匯入的用量、額度觀測、圖表歷史、診斷、註解和掃描檢查點。Codex 來源日誌不會被刪除；監控會從這些日誌目前的結尾繼續。',
  ],
  'settings.cancel': ['Cancel', '取消', '取消'],
  'settings.confirmReset': ['Confirm reset', '确认重置', '確認重設'],
  'settings.resetSuccess': [
    'Local data reset. Current weekly allowance synced; monitoring new Codex activity.',
    '本地数据已重置。当前每周额度已同步，正在监控新的 Codex 活动。',
    '本機資料已重設。目前每週額度已同步，正在監控新的 Codex 活動。',
  ],
  'settings.restoreSuccess': [
    'Pre-reset graph restored and updated with activity since the reset.',
    '重置前的图表已恢复，并加入重置后的活动。',
    '重設前的圖表已還原，並加入重設後的活動。',
  ],
  'settings.importSuccess': [
    'Full import complete. All available Codex logs were indexed.',
    '完整导入已完成。所有可用 Codex 日志均已建立索引。',
    '完整匯入已完成。所有可用 Codex 日誌均已建立索引。',
  ],
  'settings.resetFailed': [
    'Unable to reset local data: {error}',
    '无法重置本地数据：{error}',
    '無法重設本機資料：{error}',
  ],
  'settings.restoreFailed': [
    'Unable to restore the last checkpoint: {error}',
    '无法恢复上一个检查点：{error}',
    '無法還原上一個檢查點：{error}',
  ],
  'settings.importFailed': [
    'Unable to import all log data: {error}',
    '无法导入所有日志数据：{error}',
    '無法匯入所有日誌資料：{error}',
  ],
  'diagnostics.description': [
    'Aggregate health signals for local collection and estimation.',
    '本地采集和估算的汇总健康信号。',
    '本機收集和估算的彙總健康訊號。',
  ],
  'diagnostics.eventsObserved': ['Events observed', '已观测事件', '已觀測事件'],
  'diagnostics.pricedEvents': ['Priced token events', '已定价 Token 事件', '已定價 Token 事件'],
  'diagnostics.pricingPending': ['Pricing pending', '价格待定', '價格待定'],
  'diagnostics.rejected': ['Rejected observations', '已拒绝观测', '已拒絕觀測'],
  'diagnostics.partialRetries': ['Partial-line retries', '读取重试次数', '讀取重試次數'],
  'diagnostics.monitoringGaps': ['Monitoring gaps', '监控中断', '監控中斷'],
  'diagnostics.reason.partialFinalLine': ['Incomplete final line', '记录仍在写入', '記錄仍在寫入'],
  'diagnostics.reason.monitoringGap': ['Monitoring gap', '监控中断', '監控中斷'],
  'diagnostics.reason.reportedResetChanged': [
    'Reported reset changed',
    '报告的重置时间已变化',
    '回報的重設時間已變更',
  ],
  'diagnostics.reason.waitingPositivePairs': [
    'Waiting for positive paired deltas',
    '正在等待成对的正增量',
    '正在等待成對的正增量',
  ],
  'diagnostics.reason.unknownPrice': [
    'Unknown API price for model',
    '模型缺少 API 价格',
    '模型缺少 API 價格',
  ],
  'diagnostics.reason.unknownPriceForModel': [
    'No API price for {model}; add a local custom price override.',
    '模型 {model} 缺少 API 价格；请添加本地自定义价格。',
    '模型 {model} 缺少 API 價格；請新增本機自訂價格。',
  ],
  'diagnostics.modelsObserved': ['Models observed', '已观测模型', '已觀測模型'],
  'diagnostics.eligibleEvidence': ['eligible evidence', '有效证据', '有效證據'],
  'diagnostics.dataPrivacy': [
    'Prompts, account identifiers, and full local paths are never stored or returned.',
    '提示词、账户标识和完整本地路径绝不会被存储或返回。',
    '提示詞、帳戶識別資訊和完整本機路徑絕不會被儲存或傳回。',
  ],
  'diagnostics.privacyTitle': [
    'Diagnostics never include prompts, account identifiers, or full local paths.',
    '诊断信息绝不包含提示词、账户标识或完整本地路径。',
    '診斷資訊絕不包含提示詞、帳戶識別資訊或完整本機路徑。',
  ],
  'diagnostics.privacyDescription': [
    'Use this page to identify unpriced models, reset boundaries, and data-quality interruptions before relying on an estimate.',
    '在采用估算结果前，可用此页面找出未定价模型、重置边界和数据质量中断。',
    '在採用估算結果前，可用此頁面找出未定價模型、重設邊界和資料品質中斷。',
  ],
  'history.description': [
    'Finalized full-week API-equivalent estimates from valid paired token observations.',
    '根据有效的配对 Token 观测得出的完整周 API 等值估算。',
    '根據有效的配對 Token 觀測得出的完整週 API 等值估算。',
  ],
  'history.current': ['Current', '当前', '目前'],
  'history.rangeChange': ['Range change', '范围变化', '範圍變化'],
  'history.observations': ['Observations', '观测数', '觀測數'],
  'history.bucket': ['Bucket', '记录方式', '記錄方式'],
  'history.bucketRaw': ['Raw', '逐条记录', '逐筆記錄'],
  'history.allAvailable': ['All available history', '所有可用历史记录', '所有可用歷史記錄'],
  'history.completeRange': ['Complete range', '完整范围', '完整範圍'],
  'history.date': ['Date', '日期', '日期'],
  'history.estimatedValue': ['Estimated weekly value', '每周估算价值', '每週估算價值'],
  'history.observedCost': ['Observed token cost', '已观测 Token 成本', '已觀測 Token 成本'],
  'history.weeklyUsage': ['Weekly usage', '每周用量', '每週用量'],
  'history.resetWindow': ['Reset window', '重置窗口', '重設時段'],
  'history.status': ['Status', '状态', '狀態'],
  'history.weeklyWindow': ['weekly window', '每周窗口', '每週時段'],
  'history.finalized': ['Finalized', '已完成', '已完成'],
  'reset.scheduled': ['Scheduled reset', '计划重置', '排程重設'],
  'reset.uncertain': ['Uncertain reset', '重置状态不确定', '重設狀態不確定'],
  'reset.reportedChanged': [
    'Reported reset changed',
    '报告的重置时间已变化',
    '回報的重設時間已變更',
  ],
  'reset.changed': ['Reset changed', '重置已变化', '重設已變更'],
  'reset.usageDecreased': ['Usage decreased', '用量下降', '用量下降'],
  'history.pendingTitle': [
    'Pending observations are omitted',
    '待定观测不会显示',
    '待定觀測不會顯示',
  ],
  'history.pendingDescription': [
    'from the graph until a positive token-cost delta is paired with a positive weekly-usage delta.',
    '在 Token 成本正增量与每周用量正增量配对前，它们不会出现在图表中。',
    '在 Token 成本正增量與每週用量正增量配對前，它們不會出現在圖表中。',
  ],
  'starter.kicker': ['BEFORE YOU BEGIN · 01', '开始之前 · 01', '開始之前 · 01'],
  'starter.title': [
    'Help NerfTrack keep going.',
    '帮助 NerfTrack 持续发展。',
    '協助 NerfTrack 持續發展。',
  ],
  'starter.description': [
    'NerfTrack is built in the open. If it helps you understand your weekly usage, star the repository on GitHub so the project can keep growing.',
    'NerfTrack 是一个开放开发的项目。如果它帮助你了解每周用量，请在 GitHub 上为仓库加星，支持项目继续发展。',
    'NerfTrack 是一個開放開發的專案。如果它協助你瞭解每週用量，請在 GitHub 上為儲存庫加星，支持專案繼續發展。',
  ],
  'starter.githubOpened': ['GitHub opened', '已打开 GitHub', '已開啟 GitHub'],
  'starter.starGithub': [
    'Star NerfTrack on GitHub',
    '在 GitHub 上为 NerfTrack 加星',
    '在 GitHub 上為 NerfTrack 加星',
  ],
  'starter.thanks': [
    'Thanks for supporting the project.',
    '感谢你支持这个项目。',
    '感謝你支持這個專案。',
  ],
  'starter.openRepository': [
    'Open the repository and leave a star.',
    '打开仓库并点亮星标。',
    '開啟儲存庫並點亮星標。',
  ],
  'starter.linkUnavailable': [
    'The repository link will appear here once configured.',
    '配置后，仓库链接会显示在这里。',
    '設定後，儲存庫連結會顯示在這裡。',
  ],
  'starter.ready': ['Ready when you are.', '准备好后即可继续。', '準備好後即可繼續。'],
  'starter.notReady': ['Not ready to star?', '还不想加星？', '還不想加星？'],
  'starter.sadStar': [
    'The little star will be sad, but you can still continue.',
    '小星星会有点难过，但你仍然可以继续。',
    '小星星會有點難過，但你仍然可以繼續。',
  ],
  'starter.continueWithout': ['Continue without starring', '不加星标，继续', '不加星標，繼續'],
  'starter.tagline': ['Let the resets continue', '让重置继续', '讓重設繼續'],
  'starter.caption': [
    'Keep a clear eye on the week ahead.',
    '清楚掌握接下来一周。',
    '清楚掌握接下來一週。',
  ],
  'starter.saving': ['Saving…', '正在保存…', '正在儲存…'],
  'starter.continue': ['Continue to NerfTrack', '继续使用 NerfTrack', '繼續使用 NerfTrack'],
  'starter.repositoryNotConfigured': [
    'The NerfTrack GitHub repository URL has not been configured yet.',
    '尚未配置 NerfTrack GitHub 仓库地址。',
    '尚未設定 NerfTrack GitHub 儲存庫網址。',
  ],
  'home.stableValue': ['Stable Weekly API Value', '稳定的每周 API 等值', '穩定的每週 API 等值'],
  'home.earlyValue': ['Early Weekly API Value', '早期每周 API 等值', '早期每週 API 等值'],
  'home.observedTokenCost': ['Observed Token Cost', '已观测 Token 成本', '已觀測 Token 成本'],
  'home.confidence': ['Confidence', '置信度', '信賴度'],
  'home.pending': ['Pending', '待定', '待定'],
  'home.unavailable': ['Unavailable', '不可用', '無法使用'],
  'home.ofAllowance': ['of allowance', '占额度', '佔額度'],
  'home.thisWindow': ['this weekly window', '当前每周窗口', '目前每週時段'],
  'home.cumulativeEstimate': ['cumulative-window estimate', '累计窗口估算', '累計時段估算'],
  'home.validObservations': [
    '{count} valid observations',
    '{count} 个有效观测',
    '{count} 個有效觀測',
  ],
  'home.validObservation': ['1 valid observation', '1 个有效观测', '1 個有效觀測'],
  'home.needPairedDeltas': ['Need paired deltas', '需要配对增量', '需要配對增量'],
  'home.notAvailable': ['Not available', '暂无数据', '暫無資料'],
  'home.unknownCoverage': ['unknown coverage', '覆盖率未知', '涵蓋率未知'],
  'home.coverage': [
    '{value} percentage-point coverage',
    '覆盖 {value} 个百分点',
    '涵蓋 {value} 個百分點',
  ],
  'home.waitingForPair': [
    'Waiting for a positive weekly-usage change paired with local token cost.',
    '正在等待每周用量正变化与本地 Token 成本配对。',
    '正在等待每週用量正變化與本機 Token 成本配對。',
  ],
  'home.earlyProjection': [
    'Early projection {value} from {observations} and {coverage}. Waiting for more movement before calling it stable.',
    '初步预测为 {value}，来自 {observations}，{coverage}。需要更多变化才能判定为稳定。',
    '初步預測為 {value}，來自 {observations}，{coverage}。需要更多變化才能判定為穩定。',
  ],
  'home.resetsIn': ['Resets In', '距离重置', '距離重設'],
  'home.resetObserved': ['Reset observed', '已观测到重置', '已觀測到重設'],
  'home.awaitingWindow': ['Awaiting quota window', '正在等待额度窗口', '正在等待額度時段'],
  'home.live': ['Live', '实时', '即時'],
  'home.dataInterval': ['data {seconds}s', '数据 {seconds} 秒', '資料 {seconds} 秒'],
  'home.indexing': ['Indexing local data', '正在索引本地数据', '正在建立本機資料索引'],
  'home.indexingContinue': [
    'You can keep using NerfTrack while this finishes.',
    '此过程完成前，你可以继续使用 NerfTrack。',
    '此程序完成前，你可以繼續使用 NerfTrack。',
  ],
  'home.historyRange': ['History range', '历史范围', '歷史範圍'],
  'home.range.1D': ['Past Day', '过去一天', '過去一天'],
  'home.range.1W': ['Past Week', '过去一周', '過去一週'],
  'home.range.1M': ['Past Month', '过去一个月', '過去一個月'],
  'home.range.3M': ['Past 3 Months', '过去三个月', '過去三個月'],
  'home.range.6M': ['Past 6 Months', '过去六个月', '過去六個月'],
  'home.selectedRange': ['Selected range', '所选范围', '所選範圍'],
  'home.rangeUnavailable': ['{range} unavailable', '{range} 暂无数据', '{range} 暫無資料'],
  'home.since': ['Since {date}', '自 {date} 起', '自 {date} 起'],
  'home.stableDescription': [
    'Stable estimate from cumulative local usage and fetched model rates',
    '根据累计本地用量和获取的模型价格得出的稳定估算',
    '根據累計本機用量和取得的模型價格得出的穩定估算',
  ],
  'home.calibratingDescription': [
    'Calibrating from cumulative local usage and fetched model rates',
    '正在根据累计本地用量和获取的模型价格校准',
    '正在根據累計本機用量和取得的模型價格校準',
  ],
  'home.shareTitle': [
    'Browse and post in NerfTrack’s Share Your Graph discussion',
    '浏览 NerfTrack 的“分享图表”讨论并发帖',
    '瀏覽 NerfTrack 的「分享圖表」討論並發文',
  ],
  'home.shareFailed': [
    'Couldn’t open the Share Your Graph page: {error}',
    '无法打开“分享图表”页面：{error}',
    '無法開啟「分享圖表」頁面：{error}',
  ],
  'home.calibrating': ['Calibrating', '正在校准', '正在校準'],
  'home.allHistory': [
    'Showing all available log history',
    '显示所有可用日志历史',
    '顯示所有可用日誌歷史',
  ],
  'home.completeRange': ['Complete range', '完整范围', '完整範圍'],
  'home.resetAnnotations': ['Reset annotations', '重置注释', '重設註解'],
  'home.confidence.none': ['no confidence', '无置信度', '無信賴度'],
  'home.confidence.low': ['low', '低', '低'],
  'home.confidence.medium': ['medium', '中', '中'],
  'home.confidence.high': ['high', '高', '高'],
  'home.footer': [
    'Uses cumulative weekly usage and fetched API rates; short-term spikes are filtered.',
    '使用累计每周用量和获取的 API 价格，并过滤短期波动。',
    '使用累計每週用量和取得的 API 價格，並過濾短期波動。',
  ],
  'common.dayShort': ['d', '天', '天'],
  'common.hourShort': ['h', '小时', '小時'],
  'common.minuteShort': ['m', '分钟', '分鐘'],
  'chart.title': [
    'Estimated weekly API-equivalent value',
    '每周 API 等值估算',
    '每週 API 等值估算',
  ],
  'chart.subtitle': [
    'USD · local token-derived estimate',
    '美元 · 根据本地 Token 估算',
    '美元 · 根據本機 Token 估算',
  ],
  'chart.waiting': ['Waiting for weekly observations', '正在等待每周观测', '正在等待每週觀測'],
  'chart.aria': [
    'Estimated weekly API-equivalent value history chart. Use arrow keys to move between points.',
    '每周 API 等值估算历史图表。使用方向键在数据点之间移动。',
    '每週 API 等值估算歷史圖表。使用方向鍵在資料點之間移動。',
  ],
  'chart.noUsage': [
    'No activity · {duration}',
    '未观测到用量 · {duration}',
    '未觀測到用量 · {duration}',
  ],
  'chart.noActivityFor': [
    'No activity for {duration}',
    '无活动，持续 {duration}',
    '無活動，持續 {duration}',
  ],
  'chart.duration.minutes': ['{value}m', '{value} 分钟', '{value} 分鐘'],
  'chart.duration.hours': ['{value}h', '{value} 小时', '{value} 小時'],
  'chart.duration.days': ['{value}d', '{value} 天', '{value} 天'],
  'chart.duration.months': ['{value}mo', '{value} 个月', '{value} 個月'],
  'chart.resetChanges': ['{count} reset changes', '{count} 次重置变化', '{count} 次重設變化'],
  'chart.observed': ['Observed', '已观测', '已觀測'],
  'common.loading': ['Loading local state…', '正在加载本地状态…', '正在載入本機狀態…'],
  'common.localError': [
    'Local state is unavailable. Check the Diagnostics and Setup pages, then retry detection.',
    '本地状态不可用。请检查“诊断”和“设置向导”页面，然后重试检测。',
    '本機狀態無法使用。請檢查「診斷」和「設定精靈」頁面，然後重試偵測。',
  ],
} as const;

export type MessageKey = keyof typeof messages;

const localeIndex: Record<Locale, 0 | 1 | 2> = {
  'en-US': 0,
  'zh-CN': 1,
  'zh-TW': 2,
};

export function detectLocale(languages: readonly string[] = navigator.languages): Locale {
  for (const language of languages) {
    const normalized = language.toLowerCase();
    if (normalized.startsWith('zh')) {
      return /(?:-hant\b|-(?:tw|hk|mo)\b)/.test(normalized) ? 'zh-TW' : 'zh-CN';
    }
    if (normalized.startsWith('en')) return 'en-US';
  }
  return 'en-US';
}

export function translate(
  locale: Locale,
  key: MessageKey,
  values: Record<string, string | number> = {},
) {
  let message: string = messages[key][localeIndex[locale]];
  for (const [name, value] of Object.entries(values)) {
    message = message.replaceAll(`{${name}}`, String(value));
  }
  return message;
}

export function formatResetReason(locale: Locale, reason: string) {
  const normalized = reason
    .trim()
    .toLowerCase()
    .replace(/^weekly window\s*·\s*/, '')
    .replaceAll(' ', '_');
  const key: MessageKey | null =
    normalized === 'scheduled_reset'
      ? 'reset.scheduled'
      : normalized === 'uncertain_reset'
        ? 'reset.uncertain'
        : normalized === 'reported_reset_changed'
          ? 'reset.reportedChanged'
          : normalized === 'reset_changed'
            ? 'reset.changed'
            : normalized === 'usage_decreased' || normalized === 'usage_drop'
              ? 'reset.usageDecreased'
              : null;
  if (key) return translate(locale, key);
  const fallback = normalized.replaceAll('_', ' ');
  return fallback.charAt(0).toUpperCase() + fallback.slice(1);
}

export function formatDiagnosticReason(locale: Locale, reason: string) {
  const normalized = reason.trim().toLowerCase();
  const key: MessageKey | null =
    normalized === 'partial final line'
      ? 'diagnostics.reason.partialFinalLine'
      : normalized === 'monitoring gap'
        ? 'diagnostics.reason.monitoringGap'
        : normalized === 'reported reset changed'
          ? 'diagnostics.reason.reportedResetChanged'
          : normalized === 'waiting for positive paired deltas'
            ? 'diagnostics.reason.waitingPositivePairs'
            : normalized === 'unknown api price for model'
              ? 'diagnostics.reason.unknownPrice'
              : null;
  if (key) return translate(locale, key);

  const unknownPrice = reason
    .trim()
    .match(/^unknown API price for model (.+); add a local custom price override$/i);
  return unknownPrice
    ? translate(locale, 'diagnostics.reason.unknownPriceForModel', { model: unknownPrice[1] })
    : reason;
}

interface I18nValue {
  locale: Locale;
  t: (key: MessageKey, values?: Record<string, string | number>) => string;
}

const I18nContext = createContext<I18nValue>({
  locale: 'en-US',
  t: (key, values) => translate('en-US', key, values),
});

export function I18nProvider({ locale, children }: { locale: Locale; children: ReactNode }) {
  const value = useMemo<I18nValue>(
    () => ({ locale, t: (key, values) => translate(locale, key, values) }),
    [locale],
  );
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n() {
  return useContext(I18nContext);
}
