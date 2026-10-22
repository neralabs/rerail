import six
import six as s
from six import PY3

match x:
    case six.PY3:
        pass
    case [s . PY3, 1] | {"k": six.PY3.real}:
        pass
    case PY3.y | six.PY2:
        pass
    case PY3:
        pass
