with sequences as (
    select [0, 1, 1, 2] as some_numbers
    union all
    select [2, 4, 8]
)

select num
from sequences as s
cross join unnest(s.some_numbers) as num
