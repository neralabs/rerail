select
    case
      when fab <> 0 then true
      when fab < 0 then 'not a bool'
    end as fab_category
from fancy_table
