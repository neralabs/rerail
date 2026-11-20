---
"@neralabs/rerail-pack-python": patch
---

`bandit-b201-flask-debug-true` now reports `.run(debug=True)` on an app from a factory, such as `app = create_app()`, as Bandit does. After an import of a module whose name contains `flask`, any `.run` call whose `debug` keyword is `True` or `"True"` is reported. Apps made by `Flask(...)` are reported as before.
