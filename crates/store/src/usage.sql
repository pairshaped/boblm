-- func: context
SELECT CAST(context AS TEXT) AS payload, context_at AS observed_at FROM session_usage
WHERE session = @session AND context IS NOT NULL;
