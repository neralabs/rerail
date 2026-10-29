def f(*args, **kwargs):
    pass


def g(*args: int, **kwargs: str) -> None:
    pass


def h(a, *rest, b, **opts):
    pass


def i(*, a, **kw):
    pass


def j(* args, ** kwargs):
    pass


def k(
    *args,  # c
    **kwargs  # d
):
    pass


class A:
    def m(self, *args, **kwargs):
        pass

    @staticmethod
    def s(*args, **kwargs):
        pass

    def only_star(*args, **kwargs):
        pass


lam = lambda *a, **k: 0


def l(*_, **__):
    pass


def m(a, *args: "int", **kwargs: "Any"):
    pass
