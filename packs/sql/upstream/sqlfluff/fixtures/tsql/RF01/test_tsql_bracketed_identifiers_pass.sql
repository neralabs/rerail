select
    [t1].[field1],
    [t2].[field2],
    t1.field3,
    "t1".field4,
    [t3].field5,
    "t3".field6,
    t3.field7
from [table1] as [t1]
inner join [table2] as [t2]
    on [t2].[pk] = [t1].[pk]
inner join [table3] as t3
    on t3.pk = t1.pk
