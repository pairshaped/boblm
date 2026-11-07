-- Deleted chats retain call totals after their source events are removed.
-- Noon UTC preserves the recorded day in Toronto as a historical approximation.
UPDATE usage_model_call
SET created_at = unixepoch(day || ' 12:00:00') * 1000
WHERE created_at IS NULL;
