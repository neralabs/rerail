import glob
from glob import iglob as search

glob.glob("*.py", dir_fd=None)
glob.glob("*.py", dir_fd=(None))
search("*.py", dir_fd=((None)))
glob.glob("*.py", dir_fd=0)
search("*.py", dir_fd=-1)
glob.glob("*.py", **{"dir_fd": 1})
glob.glob("*.py", *[1, 2, 3])
glob.glob(dir_fd=None, pathname="*.py", recursive=True)
