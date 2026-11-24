from jinja2 import Environment
from jinja2.sandbox import SandboxedEnvironment


class TemplateEnv(SandboxedEnvironment):
    pass


template_environment = TemplateEnv(autoescape=False)
plain = Environment()
