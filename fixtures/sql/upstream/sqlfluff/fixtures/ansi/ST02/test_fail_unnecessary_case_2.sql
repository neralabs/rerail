select
    case
        when fab > 0 then false else true end as is_fab
from fancy_table
