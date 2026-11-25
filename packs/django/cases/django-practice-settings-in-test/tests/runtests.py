import sys

import django
from django.conf import settings
from django.test.utils import get_runner


# A runner script under tests/: no test calls main(), it configures its own process before the tests start.
def main():
    settings.DEBUG = False
    django.setup()
    runner = get_runner(settings)()
    sys.exit(runner.run_tests(["tests"]))


if __name__ == "__main__":
    main()
