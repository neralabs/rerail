class JobQueue:
    def add_task(self, fn, *args, **kwargs) -> None:
        fn(*args, **kwargs)


queue = JobQueue()
