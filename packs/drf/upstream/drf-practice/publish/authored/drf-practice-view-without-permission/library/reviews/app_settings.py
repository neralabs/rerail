from django.conf import settings

# Reading the setting is not assigning it.
API_SETTINGS = getattr(settings, "REST_FRAMEWORK", {})
