# The project runs with studio/ on sys.path, so its imports start below it.
from tasks.models import Task


def fill_project():
    Task.objects.filter(project_id=None).update(project_id=0)
