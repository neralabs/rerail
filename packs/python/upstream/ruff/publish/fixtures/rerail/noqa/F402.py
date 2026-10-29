import os
os.getcwd()
for os in range(3):
    pass

import os
os.getcwd()
for os in range(3):  # noqa
    pass

import os
os.getcwd()
for os in range(3):  # noqa: F402
    pass

import os
os.getcwd()
for os in range(3):  # NOQA: F402
    pass

import os
os.getcwd()
for os in range(3):  # noqa: E501
    pass
