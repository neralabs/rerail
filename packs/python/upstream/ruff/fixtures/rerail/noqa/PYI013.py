class Foo:
    ...
    value: int

class Foo:
    ...  # noqa
    value: int

class Foo:
    ...  # noqa: PYI013
    value: int

class Foo:
    ...  # NOQA: PYI013
    value: int

class Foo:
    ...  # noqa: E501
    value: int
