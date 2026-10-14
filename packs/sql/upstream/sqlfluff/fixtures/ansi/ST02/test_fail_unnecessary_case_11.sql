select
    foo,
    case
        when
            bar is not null then bar
    end as test
from baz;
