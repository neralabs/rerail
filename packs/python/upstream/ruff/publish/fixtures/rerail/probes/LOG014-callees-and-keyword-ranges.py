import logging
from logging import log as L, fatal, warn as w, exception as ex, getLogger as gl

logger = gl(__name__)

L(10, "x", exc_info=True)
fatal("x", exc_info=True)
w("x", exc_info=True)
ex("x", exc_info=True)
logger.fatal("x", exc_info=True)
logger.log(10, "x", exc_info=True)
logging.log(10, "x", exc_info=True)
logging.fatal("x", exc_info=True)
logging.warn("x", exc_info=True)
gl().warn("x", exc_info=True)
(logger.info)("x", exc_info=True)
((logger).info)("x", exc_info=True)
logger.info("x", exc_info=(True))
logger.info("x", exc_info=(  # c
    True))
logger.info("x", exc_info= True)
logger.info("x", exc_info =True)
logger.info("x", exc_info\
=True)
logger.info("x", exc_info=True,)
logger.info("x", exc_info=b"" b"a")
logger.info("x", exc_info="" "")
logger.info("x", exc_info=0 if 1 else 1)
logger.info("x", exc_info=True)  # noqa: LOG007
logger.info("x", exc_info=True)  # noqa:LOG014
logger.info("x", exc_info=True)  # NOQA
logger.info(
    "x",
    exc_info=True,  # noqa: LOG014
)
logger.info(  # noqa: LOG014
    "x",
    exc_info=True,
)
logger.info("x", exc_info=True).info("y", exc_info=True)
foo(logger.info("x", exc_info=True))
logger.info(foo(exc_info=True))
logger.info(logger.debug("x", exc_info=1), exc_info=2)
