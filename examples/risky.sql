-- A risky migration: intentionally exercises blocking and compatibility rules.
ALTER TABLE accounts ADD COLUMN last_seen_at timestamptz NOT NULL;

ALTER TABLE accounts ALTER COLUMN email TYPE citext;

CREATE INDEX idx_accounts_email ON accounts (email);

ALTER TABLE orders
    ADD CONSTRAINT orders_account_fk
    FOREIGN KEY (account_id) REFERENCES accounts (id);

ALTER TABLE accounts RENAME COLUMN email TO email_address;

BEGIN;
CREATE INDEX CONCURRENTLY idx_accounts_display_name ON accounts (display_name);
COMMIT;
