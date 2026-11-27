from jinja2.sandbox import SandboxedEnvironment

sandbox = SandboxedEnvironment()


class Notifier:
    def __init__(self):
        self.env = SandboxedEnvironment(autoescape=True)

    def render(self, body):
        first = sandbox.from_string(body).render()
        return first + self.env.from_string(body).render()
