SELECT a, "A", "Mixed Case", B.c, public.t.col, "Schema"."Table".x FROM public.t;
SELECT ÉCOLE, "ÉCOLE", école, Straße FROM t;
SELECT U&"d\0061t\+000061", U&"d!0061t" UESCAPE '!' FROM "U&t";
SELECT aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa FROM t;
SELECT "ééééééééééééééééééééééééééééééééé" FROM t;
SELECT "x""y", t.*, * FROM t, ONLY u, v AS w, LATERAL (SELECT z FROM q) s;
SELECT cat.sch.rel.col FROM cat.sch.rel;
INSERT INTO s.t (a, b) VALUES (1, 'x') ON CONFLICT (a) DO UPDATE SET b = excluded.b;
UPDATE t SET a = 1 WHERE b = 2 RETURNING c;
DELETE FROM ONLY t USING u WHERE t.id = u.id;
WITH c AS (SELECT 1 AS one) SELECT one FROM c;
CREATE TABLE "New"."T" (id int DEFAULT 42, label text DEFAULT 'none');
SELECT 1, -1, - 2, 1.5, .5, 5., 1e3, 1.5E-3, 0x1F, 0o17, 0b101, 1_000_000, 9223372036854775808, 4664.E+5;
SELECT 'a''b', E'a\nb', e'\x41\101\u0041\'', U&'d\0061t', U&'d!0061t' UESCAPE '!', $$x$$, $tag$y$$z$tag$;
SELECT 'a'
  'b', 'c' -- a comment between continuation segments
  'd';
SELECT B'1010', X'1F', N'nat', TRUE, false, NULL, 'll';
SELECT '1'::int, CAST('2' AS int), INTERVAL '1 day', DATE '2020-01-01', int '3';
SELECT 'ünïcödé', '日本語', '😀' FROM t WHERE x = 'tail';
