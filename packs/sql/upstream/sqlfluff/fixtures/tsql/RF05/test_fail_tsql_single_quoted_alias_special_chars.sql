select
    r.c.value('column[1]', 'varchar(10)') as 'Va lue'
from @xml.nodes('/rows/row') as r (c);
