"""Doc."""


class P:
    """Doc."""

    @property
    def x(self): ...

    @y.setter
    def x(self, v): ...

    @x.getter
    def x(self): ...

    @((x)).setter
    def x(self, v): ...

    @(x.deleter)
    def x(self): ...

    @self.x.setter
    def x(self, v): ...

    @x.setter
    async def x(self, v): ...

    @x.deleter
    def _x(self): ...

    @_x.setter
    def _x(self, v): ...
