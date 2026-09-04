# Token-based weekly API-equivalent estimator

NerfTrack estimates what observed local Codex activity would cost through the OpenAI API. It does not read, infer, convert, or display Codex credits, and it is not a ChatGPT bill.

For each JSONL token event, NerfTrack prices uncached input, cached input, and output at USD-per-million-token rates. When the log exposes reasoning tokens, they are treated as an output detail—not an additional billed token count—and are used only if a total output count is absent. A positive cost delta is paired only with a positive weekly `used_percent` delta in the same account/limit/reset window:

```text
cost_delta_usd = current_token_cost_usd - previous_token_cost_usd
percent_delta = current_weekly_used_percent - previous_weekly_used_percent
estimated_weekly_api_equivalent_usd = cost_delta_usd / (percent_delta / 100)
```

## Explicit Codex Fast-mode accounting

Fast mode is applied only when the rollout contains an explicit
`payload.type = "thread_settings_applied"` record with a recognized
`payload.thread_settings.service_tier`. `priority` and `fast` set the active
rollout/session mode to Fast; `default` sets it to Standard. Missing or
unrecognized values are Unknown and never become Fast. The parser carries the
most recent setting forward only to later token events in that same rollout or
session, so a tier change in one rollout does not reclassify another and events
before the first setting remain uncorrected.

The normalized event fields are persisted as `speed_mode`, `speed_source`, and
`fast_multiplier`. The exact Fast multiplier is 2.0x for the GPT-5.4 family and
GPT-6 Astra, and 2.5x for GPT-5.5, GPT-5.6, and unknown/future models with explicit Fast
evidence. Standard mode and events without explicit Fast evidence use 1.0x.
The multiplier is applied after the ordinary token-derived API cost is
calculated and before cost deltas are paired with quota-percentage deltas:

```text
effective_cost = ordinary_token_api_cost * fast_multiplier
```

NerfTrack does not divide this multiplier by Fast-mode generation speed. It
does not use graph drops, token timing, TPS, the current `config.toml`, provider
or authentication heuristics, or any other inferred signal for historical
Fast-mode accounting.

Each history point shows the unsmoothed cumulative cost-per-usage estimate for that observation. The headline remains the median of the latest seven valid cumulative estimates so short-lived noise does not redefine the current projection. Raw interval cost, percentage deltas, cumulative estimates, confidence, and coverage stay local for audit. Zero or negative cost movement, no percentage movement, unknown pricing, or a reset boundary is pending/rejected rather than converted into an estimate.

## Pricing and overrides

On every application launch, NerfTrack requests the public models.dev catalog at
https://models.dev/api.json. It reads only the direct openai.models section, so a provider's
OpenRouter resale price is not mistaken for the OpenAI API price. The catalog is downloaded and
matched locally by model ID; NerfTrack does not send prompts, token counts, account identifiers,
or usage events to models.dev.

When a valid catalog is received, its complete JSON payload and SHA-256 digest are stored in the
local database. A changed digest reprices every stored usage event and rebuilds measurements,
weekly windows, quotes, and history graphs. An HTTP 304 response reuses the cached catalog. If
models.dev is unavailable or its response fails validation, NerfTrack uses the last valid local
catalog, then the embedded fallback catalog, while preserving manual overrides.

Pricing precedence is:

1. A matching user override or alias.
2. A token-priced model in the latest valid models.dev openai.models snapshot.
3. The embedded OpenAI fallback catalog below.

Built-in fallback rates were verified on 2026-09-04 from OpenAI API model pages:

- [GPT-6 Astra](https://developers.openai.com/api/docs/models/gpt-6-astra): $10 input, $1 cached input, $50 output per 1M text tokens. Cache writes are $12.50 per 1M tokens. Prompts over 272K input tokens use 2x input/cache rates and 1.5x output; Fast mode is 2x the applicable rates.

- [GPT-5.6 Luna](https://developers.openai.com/api/docs/models/gpt-5.6-luna): $0.20 input, $0.02 cached input, $1.20 output per 1M text tokens.
- [GPT-5.6 Terra](https://developers.openai.com/api/docs/models/gpt-5.6-terra): $2 input, $0.20 cached input, $12 output per 1M text tokens.

Codex Auto Review records use the internal model ID `codex-auto-review`. NerfTrack
maps that label to GPT-5.6 Luna, so Auto Review receives Luna's full input,
cached-input, and output rates in both the live import path and the historical
reprice/rebuild path. This mapping remains available when the remote catalog is
offline and is used for future records as well as records already stored.

- [GPT-5.3-Codex](https://developers.openai.com/api/docs/models/gpt-5.3-codex): $1.75 input, $0.175 cached input, $14 output per 1M tokens.
- [GPT-5.2-Codex](https://developers.openai.com/api/docs/models/gpt-5.2-codex): $1.75 input, $0.175 cached input, $14 output per 1M tokens.
- [codex-mini-latest](https://developers.openai.com/api/docs/models/codex-mini-latest): $1.50 input, $0.375 cached input, $6 output per 1M tokens.

The embedded text fallback also covers the currently documented GPT-5.6/5.5/5.4/5.x, GPT-4.1,
GPT-4o, o1, o3, o3-mini, and o4-mini text model IDs using the [official model catalog](https://developers.openai.com/api/docs/models/all),
[model comparison](https://developers.openai.com/api/docs/models/compare), and [API pricing](https://openai.com/api/pricing/)
rates verified on that date. Token logs do not identify audio/image modality tokens, cache writes,
or tool-call units, so those non-text charges are intentionally unavailable rather than fabricated.
The official OpenAI model catalog currently documents `gpt-6-astra` but does not publish
API pricing for the screenshot's `gpt-6-astra-aeon` identifier; NerfTrack leaves that
identifier pending until an official rate is available or a local override is supplied.

User-provided model overrides are local-only and take precedence over both models.dev and the
embedded fallback. Each needs a nonempty model ID and finite non-negative input, cached-input,
and output rates; an optional alias maps a local model label to the override. A model without a
token price in any source remains conspicuously pending with a diagnostic. NerfTrack never guesses
a rate or sends model/token data to obtain one.

## Windows, reset safety, and migration

Only 10,080-minute weekly limits are used. Windows are separated by account and limit ID. Reported reset timestamps within five minutes are treated as one reset identity so normal server jitter cannot fragment a weekly allowance. A larger reset-time change or an observed scheduled boundary starts a new window. A usage regression before the reported reset is retained in raw quota history but excluded from estimation as stale/out-of-order data. Event costs are attributed by the accepted epoch's time bounds rather than exact reset timestamp equality.

Range changes are calculated only when both endpoints have medium or high confidence, the baseline lies inside the selected range, and it precedes the current estimate. Otherwise the comparison is unavailable rather than inferred from stale or low-coverage history.

Schema migration 11 preserves raw usage events, quota observations, accounts, settings, user
annotations, and checkpoints while adding normalized service-tier evidence and the Fast
multiplier audit fields. The estimator algorithm version is incremented so existing derived
estimates are invalidated. On startup NerfTrack compares the pricing digest, estimator versions,
and installed-bundle marker with the last completed rebuild. Only when one changes does it
reparse every discoverable source JSONL rollout, apply explicit historical tier corrections,
reprice token events, and rebuild measurements, estimates, windows, and graph points in one
SQLite transaction; otherwise the existing graph is retained and checkpointed collection handles
new records. A failed scan or rebuild rolls back the correction and leaves the previous graph
intact. Historical records with no explicit tier evidence remain uncorrected with an Unknown speed
mode and 1.0x multiplier; they are never upgraded to Fast. Prompts, raw
JSONL, credentials, account identifiers, and complete paths are not sent to models.dev or
returned through the UI.
