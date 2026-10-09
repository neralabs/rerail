select
    foo.a,
    bar.b
from foo
left join bar
on foo.a = bar.a
