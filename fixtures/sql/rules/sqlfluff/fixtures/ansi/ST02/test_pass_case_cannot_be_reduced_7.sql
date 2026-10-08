select
    foo,
    case
        when
            bar is null then bar
        else '123'
    end as test
from baz;
