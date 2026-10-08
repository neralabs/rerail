select
    foo,
    count( 1 )
from my_table
group by
  foo
