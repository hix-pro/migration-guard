VACUUM FULL customers;

VACUUM (FULL) audit_log;

CLUSTER customers USING idx_customers_email;

LOCK TABLE customers IN ACCESS EXCLUSIVE MODE;
