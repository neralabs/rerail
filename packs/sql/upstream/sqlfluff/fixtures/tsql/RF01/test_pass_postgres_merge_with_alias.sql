merge dw.sch.tbl dest
using land.sch.tbl src
on
    src.idx = dest.idx
    and src.name = dest.name
    and src.idy = dest.idy
when not matched by source and exists
(
    select 1 as tmp
    from land.sch.tag as ld
    where
        ld.idx = dest.idx
        and ld.name = dest.name
)
    then update set
        dest.ac = 'N'
