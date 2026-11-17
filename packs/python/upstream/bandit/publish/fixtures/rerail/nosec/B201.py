from flask import Flask
app = Flask(__name__)
app.run(debug=True)

from flask import Flask
app = Flask(__name__)
app.run(debug=True)  # nosec

from flask import Flask
app = Flask(__name__)
app.run(debug=True)  # nosec B201

from flask import Flask
app = Flask(__name__)
app.run(debug=True)  # nosec: B201

from flask import Flask
app = Flask(__name__)
app.run(debug=True)  # nosec flask_debug_true

from flask import Flask
app = Flask(__name__)
app.run(debug=True)  # nosec B101
