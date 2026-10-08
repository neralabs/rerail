select
    no_alias,
    col_b as rename_alias,
    max(sum) as max_sum
from foo
