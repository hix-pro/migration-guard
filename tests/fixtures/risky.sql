ALTER TABLE customers ADD COLUMN last_seen_at timestamptz NOT NULL;

ALTER TABLE customers ALTER COLUMN email TYPE citext;

CREATE INDEX idx_customers_email ON customers (email);

ALTER TABLE orders
    ADD CONSTRAINT orders_customer_fk
    FOREIGN KEY (customer_id) REFERENCES customers (id);

ALTER TABLE customers RENAME COLUMN email TO email_address;
