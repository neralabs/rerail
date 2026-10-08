select
    foo,
    case
        when
            bar is null then '123'
        else bar
    end as test
from baz;
