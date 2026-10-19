select
    hit_code,
    array_find(
        hit_categories_array,
        e -> e not in (
            'Apple', 'Banana', 'Cherry', 'Durian', 'Elderberry', 'Fig'
        )
    ) > -1 as is_unknown_category_flg
from hits
