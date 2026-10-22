import logging
try:
    pass
except:
    logging.error("Exception occurred", exc_info=True)

import logging
try:
    pass
except:
    logging.error("Exception occurred", exc_info=True)  # noqa

import logging
try:
    pass
except:
    logging.error("Exception occurred", exc_info=True)  # noqa: G201

import logging
try:
    pass
except:
    logging.error("Exception occurred", exc_info=True)  # NOQA: G201

import logging
try:
    pass
except:
    logging.error("Exception occurred", exc_info=True)  # noqa: E501
