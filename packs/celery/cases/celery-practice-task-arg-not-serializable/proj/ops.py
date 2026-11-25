from invoke import task


# invoke's @task, in a module that does not import celery: not a Celery task.
@task
def deploy(c, hosts):
    return hosts
