BEGIN;

CREATE INDEX CONCURRENTLY idx_customers_name ON customers (name);

COMMIT;

DROP INDEX CONCURRENTLY IF EXISTS idx_old_customers_email;

CREATE INDEX CONCURRENTLY idx_customers_email ON customers (email);
