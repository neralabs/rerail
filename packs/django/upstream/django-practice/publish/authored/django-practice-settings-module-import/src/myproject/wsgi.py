import os

from django.core.wsgi import get_wsgi_application

# manage.py, wsgi.py and asgi.py set Django up; they are left alone.
os.environ.setdefault("DJANGO_SETTINGS_MODULE", "myproject.settings")
from myproject.settings import DEBUG  # noqa: E402

application = get_wsgi_application()
