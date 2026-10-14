select
    col_a as this_alias_is_fine,
    col_b as Col_B,
    COL_C as col_c,
    Col_D as COL_D,
    col_e Col_e,
    COL_F col_f,
    Col_G COL_G,
    "col_b" as "Col_B",
    "COL_C" as "col_c",
    "Col_D" as "COL_D",
    "col_e" "Col_e",
    "COL_F" "col_f",
    "Col_G" "COL_G"
from foo
