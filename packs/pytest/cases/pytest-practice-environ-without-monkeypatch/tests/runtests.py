import os
import sys

import pytest


# A runner script under tests/: no test calls main(), it configures its own process before pytest starts.
def main():
    os.environ.setdefault("DJANGO_SETTINGS_MODULE", "shop.settings.test")
    os.environ["SHOP_RUNNER"] = "1"
    sys.exit(pytest.main(sys.argv[1:]))


if __name__ == "__main__":
    main()
