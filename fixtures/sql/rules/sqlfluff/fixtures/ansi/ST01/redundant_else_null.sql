select
    case
        when name = 'cat' then 'meow'
        when name = 'dog' then 'woof'
        else null
    end
from x
