select
  a,
  b
from tbl
union distinct
select
  c,
  d
from tbl1
union
select
  e,
  f
from tbl2
