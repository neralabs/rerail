from svc.services.http import fetch


class Provider:
    def models(self):
        return []

    async def models_async(self):
        # Provider.models does not block; a subclass's override runs only for that subclass.
        return self.models()


class Ollama(Provider):
    def models(self):
        return fetch("http://localhost:11434/api/tags")

    async def models_async(self):
        return [m for m in self.models()]  # caught
