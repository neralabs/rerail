from .models import Book


class BaseConsumer:
    async def receive(self, text):
        return self.handle(text)

    def handle(self, text):
        return text.upper()


class CountingConsumer(BaseConsumer):  # caught
    def handle(self, text):
        return Book.objects.filter(title=text).count()


class Direct:
    async def receive(self, text):
        return self.lookup(text)  # caught

    def lookup(self, text):
        return self.fetch(text)

    def fetch(self, text):
        return Book.objects.get(title=text)


class Plain(Direct):
    def fetch(self, text):
        return text
