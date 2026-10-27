DO $$
BEGIN
  DELETE FROM customers;
END
$$;

CALL cleanup_old_sessions();
