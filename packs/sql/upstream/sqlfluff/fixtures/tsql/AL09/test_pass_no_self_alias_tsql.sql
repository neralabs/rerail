select
    no_alias,
    rename_alias = col_b,
    max_sum = max(sum)
from foo
