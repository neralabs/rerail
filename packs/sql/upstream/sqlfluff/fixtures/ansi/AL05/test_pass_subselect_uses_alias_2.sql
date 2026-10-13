select
  COL_A
  , COL_B
from INSERTS INS
where COL_B != (select max(COL_B) from INSERTS X where INS.COL_A = X.COL_A)
