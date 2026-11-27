REST_FRAMEWORK = {
    "DEFAULT_PERMISSION_CLASSES": [
        "library.api.permissions.HasObjectPermission",
        "rest_framework.permissions.IsAuthenticated",
    ],
}
