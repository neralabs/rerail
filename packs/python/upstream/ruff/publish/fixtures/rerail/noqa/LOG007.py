import logging
logger = logging.getLogger(__name__)
logger.exception("x", exc_info=False)

import logging
logger = logging.getLogger(__name__)
logger.exception("x", exc_info=False)  # noqa

import logging
logger = logging.getLogger(__name__)
logger.exception("x", exc_info=False)  # noqa: LOG007

import logging
logger = logging.getLogger(__name__)
logger.exception("x", exc_info=False)  # NOQA: LOG007

import logging
logger = logging.getLogger(__name__)
logger.exception("x", exc_info=False)  # noqa: E501
