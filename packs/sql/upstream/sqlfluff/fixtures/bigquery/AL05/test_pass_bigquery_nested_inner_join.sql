with abh as (
    select
        ceb.emailaddresskey,
        dac.accountkey
    from table2 as dac
    inner join table3 as ceb
        on ceb.col2 = dac.col2
)
select col1
from table1 as abg
inner join  abh
on abg.col1 = abh.col1
