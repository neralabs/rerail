select
    col_a as this_alias_is_fine,
    "col_b" as col_b,
    COL_C as "COL_C",
    "Col_D" as Col_D,
    col_e "col_e",
    "COL_F" COL_F,
    Col_G "Col_G"
from foo
