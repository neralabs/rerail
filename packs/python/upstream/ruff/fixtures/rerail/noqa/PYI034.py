class C:
    def __new__(cls) -> C: ...

class C:
    def __new__(cls) -> C: ...  # noqa

class C:
    def __new__(cls) -> C: ...  # noqa: PYI034

class C:
    def __new__(cls) -> C: ...  # NOQA: PYI034

class C:
    def __new__(cls) -> C: ...  # noqa: E501
