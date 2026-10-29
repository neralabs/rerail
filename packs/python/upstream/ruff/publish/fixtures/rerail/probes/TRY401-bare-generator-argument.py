import logging

logger = logging.getLogger(__name__)


def f(y):
    try:
        pass
    except Exception as e:
        logger.exception(str(x) for x in e)
        logger.exception(e for _ in y)
        logger.exception(x for x in y if e)
