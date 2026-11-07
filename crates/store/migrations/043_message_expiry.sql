-- A message that waited too long expires unsent (ADR 0004). The delivery_state CHECK would need a
-- table rebuild to admit a new value, which live runners write to, so expiry is a separate marker.
-- Readers report an expired row as delivery state "expired"; expired_from records whether it was
-- queued or unconfirmed when it expired.
ALTER TABLE messages ADD COLUMN expired_at INTEGER;
ALTER TABLE messages ADD COLUMN expired_from TEXT CHECK (expired_from IN ('queued', 'uncertain'));
