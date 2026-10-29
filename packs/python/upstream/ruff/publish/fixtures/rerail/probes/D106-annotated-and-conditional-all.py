__all__: list[str] = ["Exported"]
if True:
    __all__ += ["Cond"]


class Exported:
    class Inner: ...


class NotExported:
    class Inner: ...


if True:
    class Cond:
        class Inner: ...
