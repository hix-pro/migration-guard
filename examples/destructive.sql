-- A destructive migration: every statement removes or rewrites data.
BEGIN;

DROP TABLE legacy_sessions;

ALTER TABLE accounts DROP COLUMN phone;

TRUNCATE audit_log;

DELETE FROM sessions;

UPDATE accounts SET is_active = false;

COMMIT;
