select
    foo,
    case
        when
            bar is null then null
        else bar
    end as test
from baz;
