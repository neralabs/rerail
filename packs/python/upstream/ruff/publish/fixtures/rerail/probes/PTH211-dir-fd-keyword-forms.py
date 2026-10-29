import os
from os import symlink as link

os.symlink("a", "b", dir_fd=None)
os.symlink("a", "b", dir_fd=(None))
link("a", "b", dir_fd=((None)))
os.symlink("a", "b", dir_fd=0)
os.symlink("a", "b", dir_fd=-1, target_is_directory=True)
os.symlink("a", "b", **{"dir_fd": 1})
os.symlink("a", "b", *[True, 3])
(os.symlink)(src="a", dst="b")
os.symlink()
