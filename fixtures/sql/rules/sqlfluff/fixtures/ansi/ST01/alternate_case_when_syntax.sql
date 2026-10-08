select
    case name
        when 'cat' then 'meow'
        when 'dog' then 'woof'
        else null
    end
from x
