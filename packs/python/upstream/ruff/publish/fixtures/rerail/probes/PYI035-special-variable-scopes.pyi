class Foo:
    if True:
        __slots__: tuple[str, ...]
        __match_args__: tuple[str, ...]
    else:
        __slots__: tuple[str, ...] = ()
    def method(self) -> None:
        __slots__: tuple[str, ...]
        __all__: list[str]
    class Inner:
        __slots__: tuple[str, ...]
        __all__: list[str]
