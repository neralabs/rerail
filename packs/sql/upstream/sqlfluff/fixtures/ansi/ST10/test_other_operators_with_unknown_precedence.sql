with
cte as (
    select 1 as num
    union all
    select 2 as num
    union all
    select 3 as num
    union all
    select 4 as num
)

select
    *,
    case
        when num % 2 = 0 then 1
        else 2
    end as test
from cte
