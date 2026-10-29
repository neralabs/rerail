import logging
logger = logging.getLogger(__name__)
logger.info("x", exc_info=True)

import logging
logger = logging.getLogger(__name__)
logger.info("x", exc_info=True)  # noqa

import logging
logger = logging.getLogger(__name__)
logger.info("x", exc_info=True)  # noqa: LOG014

import logging
logger = logging.getLogger(__name__)
logger.info("x", exc_info=True)  # NOQA: LOG014

import logging
logger = logging.getLogger(__name__)
logger.info("x", exc_info=True)  # noqa: E501
