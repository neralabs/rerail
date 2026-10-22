from collections import namedtuple
class A(namedtuple("A", "x")):
    pass

from collections import namedtuple
class A(namedtuple("A", "x")):  # noqa
    pass

from collections import namedtuple
class A(namedtuple("A", "x")):  # noqa: SLOT002
    pass

from collections import namedtuple
class A(namedtuple("A", "x")):  # NOQA: SLOT002
    pass

from collections import namedtuple
class A(namedtuple("A", "x")):  # noqa: E501
    pass
