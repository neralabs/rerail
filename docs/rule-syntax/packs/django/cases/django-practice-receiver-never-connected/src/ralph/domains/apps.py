from ralph.apps import RalphAppConfig


class Domains(RalphAppConfig):
    name = "ralph.domains"

    def get_load_modules_when_ready(self):
        return ["publishers"]
