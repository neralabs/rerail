from celery import Celery

# SQS, no visibility_timeout: the queue's default of 30 minutes applies.
app = Celery("proj", broker="sqs://")
