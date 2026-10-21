INSERT INTO t (x, y) SELECT s.a, s.b FROM s ON CONFLICT (x) DO UPDATE SET y = excluded.y;
