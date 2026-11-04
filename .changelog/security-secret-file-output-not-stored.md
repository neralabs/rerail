The output of a tool call that names a secret-like file (`cat .env`) is no longer stored, and Rerail keeps no copy of such a file's text before an edit, so it cannot roll one back.
