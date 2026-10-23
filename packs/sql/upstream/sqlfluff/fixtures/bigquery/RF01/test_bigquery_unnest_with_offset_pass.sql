select
    ix,
    v
from t
left join
    unnest(t.value_list) as v
    with offset as ix
