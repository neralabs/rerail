select cc.c1
from (
        select table1.c1
        from
            table1
        inner join
            table2
            on table1.x_id = table2.x_id
        inner join
            table3
            on table2.y_id = table3.y_id
) as cc
