select
    case
        when fab > 0 and tot > 0 then true else false end as is_fab
from fancy_table
