import logging
try:
    pass
except:
    logging.exception("Exception occurred", exc_info=True)

import logging
try:
    pass
except:
    logging.exception("Exception occurred", exc_info=True)  # noqa

import logging
try:
    pass
except:
    logging.exception("Exception occurred", exc_info=True)  # noqa: G202

import logging
try:
    pass
except:
    logging.exception("Exception occurred", exc_info=True)  # NOQA: G202

import logging
try:
    pass
except:
    logging.exception("Exception occurred", exc_info=True)  # noqa: E501
