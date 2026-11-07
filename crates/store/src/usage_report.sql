-- func: token_daily
SELECT provider, model, day, input_tokens, cache_read_tokens, cache_write_tokens, output_tokens, calls
FROM usage_token_daily
WHERE day >= @since AND day <= @until
ORDER BY day, provider, model;

-- func: allowance_daily
SELECT provider, account_id, day, weekly_used, weekly_resets_at, observed_at, samples
FROM usage_allowance_daily
WHERE day >= @since AND day <= @until
ORDER BY provider, account_id, day;

-- func: token_calls
SELECT provider, model, COALESCE(created_at, 0) AS created_at, input_tokens, cache_read_tokens, cache_write_tokens, output_tokens
FROM usage_model_call
WHERE created_at IS NOT NULL AND created_at >= @since AND created_at < @until
ORDER BY created_at;

-- func: allowance_observations
SELECT provider, account_id, weekly_used, weekly_resets_at, observed_at
FROM usage_allowance_observation
WHERE observed_at >= @since AND observed_at < @until
ORDER BY observed_at, seq, session;
