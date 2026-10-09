select
    case
        when not fab > 0 or tot > 0 then false else true end as is_fab
from fancy_table
