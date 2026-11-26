from . import settings  # caught
from .settings import DEBUG  # caught

urlpatterns = []
if DEBUG or settings.SITE_URL:
    urlpatterns = []
