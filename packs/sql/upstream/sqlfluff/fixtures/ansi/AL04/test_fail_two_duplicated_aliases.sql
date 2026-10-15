select 1
from table_1 as a
join table_2 as a on a.pk = b.pk
join table_3 as b on a.pk = b.pk
join table_4 as b on b.pk = b.pk
