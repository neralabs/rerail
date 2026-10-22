class C:
    def __exit__(self, *args: int) -> None: ...

class C:
    def __exit__(self, *args: int) -> None: ...  # noqa

class C:
    def __exit__(self, *args: int) -> None: ...  # noqa: PYI036

class C:
    def __exit__(self, *args: int) -> None: ...  # NOQA: PYI036

class C:
    def __exit__(self, *args: int) -> None: ...  # noqa: E501
