select
    case name
        when 'cat' then 'meow'
        when 'dog' then 'woof'
        else iff(wing_type is not null, 'tweet', 'invalid')
    end
from x
