from dynaconf import Dynaconf

# Another library's settings object, also named `settings`.
settings = Dynaconf(settings_files=["settings.toml"])
