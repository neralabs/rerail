import os

from django.core.wsgi import get_wsgi_application

# Not test code: the application configures its own process.
os.environ.setdefault("DJANGO_SETTINGS_MODULE", "shop.settings")
os.environ["SHOP_WSGI"] = "1"

application = get_wsgi_application()
