CREATE INDEX idx_customers_email ON customers (email);

DROP INDEX idx_customers_email ON customers;

ALTER TABLE orders
    ADD CONSTRAINT orders_customer_fk
    FOREIGN KEY (customer_id) REFERENCES customers (id);

ALTER TABLE customers ADD COLUMN last_seen_at timestamp NOT NULL;

ALTER TABLE customers RENAME COLUMN email TO email_address;
