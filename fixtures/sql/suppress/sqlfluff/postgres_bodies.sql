CREATE FUNCTION f() RETURNS SETOF int LANGUAGE sql AS $$
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa
$$;
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa: CV05
SELECT $$ -- noqa $$ FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
SELECT E'\' -- noqa' FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
SELECT "-- noqa" FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
/* outer /* nested */ -- noqa */ SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; /* noqa:
disable=CV05 */
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
