DROP TABLE legacy_sessions;

ALTER TABLE customers DROP COLUMN phone;

TRUNCATE audit_log;

DELETE FROM sessions;

UPDATE customers SET is_active = false;
