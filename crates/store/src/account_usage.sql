-- func: session_account
SELECT agent AS provider, account_id FROM sessions WHERE id = @session;

-- func: get
SELECT provider, account_id,
 five_hour_used, five_hour_resets_at, five_hour_observed_at,
 weekly_used, weekly_resets_at, weekly_observed_at,
 credits_balance, credits_observed_at
FROM account_usage WHERE provider = @provider AND account_id = @account_id;

-- func: all
SELECT provider, account_id,
 five_hour_used, five_hour_resets_at, five_hour_observed_at,
 weekly_used, weekly_resets_at, weekly_observed_at,
 credits_balance, credits_observed_at
FROM account_usage ORDER BY provider, account_id;

-- func: by_provider
SELECT provider, account_id,
 five_hour_used, five_hour_resets_at, five_hour_observed_at,
 weekly_used, weekly_resets_at, weekly_observed_at,
 credits_balance, credits_observed_at
FROM account_usage WHERE provider = @provider ORDER BY account_id;

-- func: insert
INSERT INTO account_usage(
 provider, account_id,
 five_hour_used, five_hour_resets_at, five_hour_observed_at,
 weekly_used, weekly_resets_at, weekly_observed_at,
 credits_balance, credits_observed_at)
VALUES(
 @provider, @account_id,
 @five_hour_used, @five_hour_resets_at, @five_hour_observed_at,
 @weekly_used, @weekly_resets_at, @weekly_observed_at,
 @credits_balance, @credits_observed_at);

-- func: update
UPDATE account_usage SET
 five_hour_used = @five_hour_used,
 five_hour_resets_at = @five_hour_resets_at,
 five_hour_observed_at = @five_hour_observed_at,
 weekly_used = @weekly_used,
 weekly_resets_at = @weekly_resets_at,
 weekly_observed_at = @weekly_observed_at,
 credits_balance = @credits_balance,
 credits_observed_at = @credits_observed_at
WHERE provider = @provider AND account_id = @account_id;

-- func: sessions_on_account
SELECT id FROM sessions
WHERE agent = @provider AND account_id = @account_id AND archived = 0
ORDER BY id;

-- func: latest_state_summary
SELECT payload FROM events
WHERE session = @session AND kind = 'state_summary'
ORDER BY seq DESC LIMIT 1;
