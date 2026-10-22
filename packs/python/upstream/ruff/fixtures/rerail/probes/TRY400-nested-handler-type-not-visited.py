import logging

logger = logging.getLogger(__name__)


def f():
    try:
        pass
    except Exception:
        try:
            pass
        except (logger.error("t") or ValueError):
            logger.error("inner body")
        except (lambda: logger.error("u"))() as e:
            pass
