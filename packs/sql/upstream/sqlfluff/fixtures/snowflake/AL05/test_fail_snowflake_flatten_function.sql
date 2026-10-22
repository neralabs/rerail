SELECT
    r.rec:foo::string,
    value:bar::string
FROM foo.bar AS r, LATERAL FLATTEN(input => rec:result) AS x
