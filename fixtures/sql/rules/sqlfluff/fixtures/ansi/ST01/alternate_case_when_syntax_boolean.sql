select
    case name
        when 'cat' then true
        when 'dog' then true
        else name is null
    end
from x
