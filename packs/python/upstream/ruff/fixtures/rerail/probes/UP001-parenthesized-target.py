class A:
    (__metaclass__) = type
    ((__metaclass__)) = (  # comment
        type
    )
    __metaclass__ = \
        type
    __metaclass__: type = type
    (__metaclass__,) = type
    a = __metaclass__ = type
    __metaclass__ = type; x = 1
    __metaclass__ = (type)  # noqa: UP001


__metaclass__ = type
