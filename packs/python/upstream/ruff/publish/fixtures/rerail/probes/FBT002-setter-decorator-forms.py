class C:
    @property
    def foo(self) -> bool:
        return True

    @foo.setter
    def foo(self, value: bool = False) -> None:
        pass

    @(foo).setter
    def foo(self, value: bool = True) -> None:
        pass

    @((foo)).setter
    def foo(self, value: bool = True) -> None:
        pass

    @bar.setter
    def foo(self, value: bool = False) -> None:
        pass

    @self.foo.setter
    def foo(self, value: bool = True) -> None:
        pass

    @(foo.bar).setter
    def foo(self, value: bool = True) -> None:
        pass

    @foo.setter()
    def foo(self, value: bool = True) -> None:
        pass

    @x.setter
    def setter(self, value: bool = True) -> None:
        pass

    @foo.getter
    def foo(self, value: bool = True) -> None:
        pass
