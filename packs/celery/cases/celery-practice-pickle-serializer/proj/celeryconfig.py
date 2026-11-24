# A celeryconfig module: bare lowercase keys are Celery's settings.
task_serializer = "pickle"  # caught
accept_content = ["pickle"]  # caught
result_serializer = "yaml"
result_accept_content = ["json"]
