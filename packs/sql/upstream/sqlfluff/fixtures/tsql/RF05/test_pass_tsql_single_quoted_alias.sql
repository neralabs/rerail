select
    r.c.value('column[1]', 'varchar(10)') as 'Value'
from @xml.nodes('/rows/row') as r (c);
