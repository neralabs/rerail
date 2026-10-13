select
    sc.col1 as colx
    , pn.col1 as coly
from sch1.tbl1 as sc
cross join unnest(array[111, 222]) as pn(col1)
