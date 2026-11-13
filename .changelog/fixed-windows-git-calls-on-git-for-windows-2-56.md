On Windows with Git for Windows 2.56, Rerail starts Git's own `git.exe` in `ucrt64\bin` instead of the launcher in `cmd`, so each Git call Rerail makes is one process instead of two.
