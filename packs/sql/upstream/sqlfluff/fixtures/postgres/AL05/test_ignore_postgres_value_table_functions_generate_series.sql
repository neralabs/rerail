SELECT
  date_trunc('day', dd):: timestamp with time zone
FROM generate_series (
    '2022-02-01'::timestamp , NOW()::timestamp , '1 day'::interval
) dd ;
