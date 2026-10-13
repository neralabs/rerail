SELECT a
FROM json_array_elements('[{"id": "a1"}]') AS a,
    json_array_elements('[{"id": "A1"}, {"id": "A2"}]') AS b
WHERE lower(a ->> 'id') = lower(b ->> 'id');
