select
    a.complex.key,
    a.complex.structure.val
from sch.table1 as t1
left join sch.table2 as t2
on t1.id = t2.id
