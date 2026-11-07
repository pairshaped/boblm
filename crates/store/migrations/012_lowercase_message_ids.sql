-- Message IDs are compared in lowercase (hyphenated) everywhere now. Lowercase the ones
-- stored earlier. A message whose lowercase twin already exists keeps its spelling, so no
-- two messages are merged and no row is lost.
-- Rows that point at a message are updated in the same transaction; foreign keys are checked
-- at commit so the order of the updates below does not matter.
PRAGMA defer_foreign_keys = ON;
UPDATE OR IGNORE messages SET uuid = lower(uuid) WHERE uuid <> lower(uuid);
-- Uploads attached to a renamed message follow it.
UPDATE attachments SET message_uuid = lower(message_uuid)
WHERE message_uuid IS NOT NULL
  AND message_uuid <> lower(message_uuid)
  AND message_uuid NOT IN (SELECT uuid FROM messages);
-- The events that refer to a renamed message follow it, so the chat keeps matching them.
UPDATE events SET payload = json_set(payload, '$.uuid', lower(json_extract(payload, '$.uuid')))
WHERE kind IN ('message_appended', 'delivery_changed', 'delivery_rejected')
  AND json_extract(payload, '$.uuid') IS NOT NULL
  AND json_extract(payload, '$.uuid') <> lower(json_extract(payload, '$.uuid'))
  AND json_extract(payload, '$.uuid') NOT IN (SELECT uuid FROM messages);
