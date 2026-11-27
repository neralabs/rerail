from app.templating import plain, template_environment


def render_title(title):
    return plain.from_string(title).render()  # caught


def render_path(fmt):
    return template_environment.from_string(fmt).render()
