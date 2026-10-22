from itertools import starmap
x = [1, 2]
y = zip(x, x[1:])  

from itertools import starmap
x = [1, 2]
y = zip(x, x[1:])    # noqa

from itertools import starmap
x = [1, 2]
y = zip(x, x[1:])    # noqa: RUF007

from itertools import starmap
x = [1, 2]
y = zip(x, x[1:])    # NOQA: RUF007

from itertools import starmap
x = [1, 2]
y = zip(x, x[1:])    # noqa: E501
