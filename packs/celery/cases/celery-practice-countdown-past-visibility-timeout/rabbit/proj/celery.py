from celery import Celery

app = Celery("proj", broker="amqp://guest@localhost//")
