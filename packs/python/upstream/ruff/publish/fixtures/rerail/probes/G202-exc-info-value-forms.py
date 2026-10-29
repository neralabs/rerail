import logging
import sys
import sys as s
from sys import exc_info as ei

try:
    pass
except:
    logging.error("a", exc_info=(True))
    logging.exception("a", exc_info=(True))
    logging.error("b", exc_info=(sys.exc_info()))
    logging.exception("b", exc_info=(sys.exc_info()))
    logging.error("c", exc_info=s.exc_info())
    logging.exception("c", exc_info=s.exc_info())
    logging.error("d", exc_info=ei())
    logging.exception("d", exc_info=ei())
    logging.error("e", exc_info=1)
    logging.exception("e", exc_info=1)
    logging.error("f", exc_info=e)
    logging.exception("f", exc_info=e)
    logging.error("g", exc_info=None)
    logging.exception("g", exc_info=None)
    logging.error("h", exc_info=not False)
    logging.exception("h", exc_info=not False)
    logging.error("i", exc_info=sys.exc_info)
    logging.exception("i", exc_info=sys.exc_info)
    logging.error("j", exc_info=True if x else False)
    logging.exception("j", exc_info=True if x else False)
    logging.error("k", exc_info=(sys.exc_info)())
    logging.exception("k", exc_info=(sys.exc_info)())
    logging.error("l", exc_info=sys.exc_info(1))
    logging.exception("l", exc_info=sys.exc_info(1))
    logging.error("m", **{"exc_info": True})
    logging.exception("m", **{"exc_info": True})
    logging.error("n", True)
    logging.exception("n", True)
