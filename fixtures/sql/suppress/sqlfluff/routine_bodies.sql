CREATE FUNCTION f() RETURNS int LANGUAGE sql AS $$
-- noqa: disable=all
SELECT 1;
$$;
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
CREATE FUNCTION g() RETURNS int LANGUAGE sql AS $body$ SELECT 1; /* noqa: disable=all */ $body$;
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
CREATE FUNCTION h() RETURNS int LANGUAGE sql
BEGIN ATOMIC
-- noqa: disable=CV08
SELECT 1;
END;
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
CREATE FUNCTION k() RETURNS int LANGUAGE sql AS -- noqa: disable=CV05
$$ SELECT 1 $$;
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
