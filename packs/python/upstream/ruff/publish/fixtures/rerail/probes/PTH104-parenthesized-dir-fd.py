import os
from os import rename as mv

mv("a", "b", src_dir_fd=(None))
mv("c", "d", dst_dir_fd=3)
os.rename("e", "f", src_dir_fd=None, dst_dir_fd=(None))
os.rename("g", "h", src_dir_fd=None, dst_dir_fd=fd)
os.rename(*["i", "j"])
os.rename("k", "l", **{"src_dir_fd": 3})
os.rename()
