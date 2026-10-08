SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa: CV05
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa:CV05,CV08
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa: CV05, CV08
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; --noqa: convention.is_null
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa: L049
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa: CV*
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa: convention
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa: core
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa: all
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa: cv05
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- NOQA
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa:
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- a reason -- noqa: CV05
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa: CV05 -- a reason
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; /* noqa */
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; /* noqa: CV05 */
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL /* noqa: CV05 */;
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa CV05
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa: XX99
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa: disable
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa: PRS
SELECT '-- noqa' FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
-- noqa
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL;
SELECT a
FROM t RIGHT JOIN u ON t.x = u.x -- noqa: CV05
WHERE b = NULL; -- noqa: CV05
SELECT a FROM t RIGHT JOIN u ON t.x = u.x WHERE b = NULL; -- noqa: CV05,
