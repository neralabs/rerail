from invoke import task


@task
def deploy(c):
    return c


def later(c):
    deploy.apply_async(countdown=7200)
