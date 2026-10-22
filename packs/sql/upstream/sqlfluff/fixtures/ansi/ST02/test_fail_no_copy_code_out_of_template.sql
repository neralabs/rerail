select
    foo,
    case
        when
            bar is null then {{ result }}
        else bar
    end as test
from baz;
