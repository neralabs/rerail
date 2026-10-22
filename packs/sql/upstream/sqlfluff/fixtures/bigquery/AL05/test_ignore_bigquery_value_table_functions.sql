select *
from unnest(generate_timestamp_array(
    '2020-01-01', '2020-01-30', interval 1 day)) as ts
