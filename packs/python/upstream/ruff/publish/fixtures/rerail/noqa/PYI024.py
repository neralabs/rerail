from collections import namedtuple
X = namedtuple("X", ["a"])

from collections import namedtuple
X = namedtuple("X", ["a"])  # noqa

from collections import namedtuple
X = namedtuple("X", ["a"])  # noqa: PYI024

from collections import namedtuple
X = namedtuple("X", ["a"])  # NOQA: PYI024

from collections import namedtuple
X = namedtuple("X", ["a"])  # noqa: E501
