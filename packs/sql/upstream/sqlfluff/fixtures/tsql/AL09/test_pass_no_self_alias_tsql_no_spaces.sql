select
    no_alias,
    col_b=rename_alias,
    rename_alias_2 AS col_c,
    max_sum=max(sum)
from foo
