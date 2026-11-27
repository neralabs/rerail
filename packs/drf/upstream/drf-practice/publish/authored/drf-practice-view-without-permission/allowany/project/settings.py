from rest_framework.permissions import AllowAny

# The default is set, to AllowAny only: every view without its own permission classes stays open.
REST_FRAMEWORK = {
    "DEFAULT_PERMISSION_CLASSES": (AllowAny,),
}
