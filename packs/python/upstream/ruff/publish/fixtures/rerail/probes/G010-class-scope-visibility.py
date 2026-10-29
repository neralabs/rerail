class C:
    from foo import logger
    logger.warn("a")

    def m(self):
        logger.warn("b")

    LOG = make()

    def n(self):
        LOG.warn("c")

    from logging import warn

    def p(self):
        warn("d")

    warn("e")
