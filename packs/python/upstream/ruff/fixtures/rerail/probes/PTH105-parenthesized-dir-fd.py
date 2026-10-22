import os
from os import replace as mv

mv("a", "b", src_dir_fd=(None))
mv("c", "d", dst_dir_fd=3)
os.replace("e", "f", src_dir_fd=None, dst_dir_fd=(None))
os.replace("g", "h", src_dir_fd=None, dst_dir_fd=fd)
os.replace(*["i", "j"])
os.replace("k", "l", **{"src_dir_fd": 3})
os.replace()
