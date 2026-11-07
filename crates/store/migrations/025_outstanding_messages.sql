-- Dispatch only needs unresolved inputs, not the completed conversation history.
CREATE INDEX messages_outstanding ON messages(session, seq)
WHERE delivery_state IN ('queued', 'dispatching', 'uncertain', 'rejected');
