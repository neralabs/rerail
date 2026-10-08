select
    foo,
    case
        when
            bar is not null then '123'
        when
            foo is not null then '456'
        else bar
    end as test
from baz;
