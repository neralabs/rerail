import logging
try:
    pass
except:
    class K:
        f = lambda: logging.error("a", exc_info=True)
        g = [logging.exception(i, exc_info=True) for i in x]

    @dec(lambda: logging.error("b", exc_info=True))
    class L:
        pass

    def h(x=lambda: logging.exception("c", exc_info=True)):
        pass

    def k() -> logging.error("d", exc_info=True):
        pass

    def one(): logging.error("e", exc_info=True)
    class Two: logging.exception("f", exc_info=True)
