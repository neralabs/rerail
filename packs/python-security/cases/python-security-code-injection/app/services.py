import jinja2
from jinja2.sandbox import SandboxedEnvironment

SAVED_TEMPLATE = "Dear {{ name }}, your order shipped."
_env = jinja2.Environment(undefined=jinja2.StrictUndefined)


def get_settings():
    return None


def render_prompt(template, variables):
    return _env.from_string(template).render(**variables)  # caught


def render_saved(variables):
    return _env.from_string(SAVED_TEMPLATE).render(**variables)


def render_sandboxed(template, variables):
    return SandboxedEnvironment().from_string(template).render(**variables)
