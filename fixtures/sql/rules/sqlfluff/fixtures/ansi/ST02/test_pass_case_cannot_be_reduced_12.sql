select
    field_1,
    field_2,
    field_3,
    case
        when coalesce(field_2, field_3) is null then 1 else 0
    end as field_4
from my_table;
