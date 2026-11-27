from celery import Celery, Task
from sqlalchemy import inspect

from wrapped.models import Base


class ModelRef:
    def __init__(self, obj):
        self.key = inspect(obj).identity_key[:2]


class WrappingTask(Task):
    def apply_async(self, args=None, kwargs=None, **options):
        if args is not None:
            args = tuple(ModelRef(a) if isinstance(a, Base) else a for a in args)
        return super().apply_async(args=args, kwargs=kwargs, **options)


app = Celery("wrapped", task_cls=WrappingTask)
