from gettext import gettext as _
name = "x"
_(f"hello {name}")

from gettext import gettext as _
name = "x"
_(f"hello {name}")  # noqa

from gettext import gettext as _
name = "x"
_(f"hello {name}")  # noqa: INT001

from gettext import gettext as _
name = "x"
_(f"hello {name}")  # NOQA: INT001

from gettext import gettext as _
name = "x"
_(f"hello {name}")  # noqa: E501
