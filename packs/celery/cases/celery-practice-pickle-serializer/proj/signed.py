# A custom serializer that signs pickle with an HMAC (paperless-ngx's shape): left alone.
CELERY_RESULT_SERIALIZER = "signed-pickle"
CELERY_TASK_SERIALIZER = "signed-pickle"
CELERY_ACCEPT_CONTENT = ["application/json", "application/x-signed-pickle"]
