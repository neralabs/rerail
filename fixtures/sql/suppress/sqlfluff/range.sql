SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
-- noqa: disable=CV05
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
-- noqa: enable=CV05
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
-- noqa: disable=all
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
-- noqa: enable=all
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
-- noqa: disable=CV05,CV08
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
-- noqa: enable=CV08
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
-- noqa: enable=all
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa: disable=CV05
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
SELECT a FROM t RIGHT JOIN u ON t.x = u.x /* noqa: enable=CV05 */ WHERE b = NULL;
SELECT a FROM t RIGHT JOIN u ON t.x = u.x /* noqa: disable=CV05 */ WHERE b = NULL;
/* noqa: enable=all */
-- noqa: disable=convention.is_null
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
-- noqa: enable=CV05
-- noqa: disable=CV*
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
-- noqa: enable=all
-- noqa: enable=CV08
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
-- noqa: disable=CV08
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
