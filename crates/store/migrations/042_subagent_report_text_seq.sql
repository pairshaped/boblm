-- The assistant text event a delivered report was built from, so a later result with no newer
-- text is recognized as a repeat. Add-only: a nullable column, so live old runners that insert
-- reports through the unchanged queue trigger keep working.
ALTER TABLE subagent_reports ADD COLUMN delivered_text_seq INTEGER;
