# An Open edX style plugin hook receives the settings module being built as a parameter: not django.conf.settings.
def plugin_settings(settings):
    settings.ANALYTICS_URL = None
    settings.FEATURES["X"] = True


class Config:
    def __init__(self):
        self.settings = {}

    def set(self):
        self.settings.DEBUG = True
