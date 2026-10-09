select
    foo,
    case
        when
            bar is not null and abs(foo) > 0 then '123'
        else bar
    end as test
from baz;
