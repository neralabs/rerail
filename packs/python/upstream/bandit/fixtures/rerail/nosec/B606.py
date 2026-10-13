import os
os.execl('/bin/ls', 'ls')

import os
os.execl('/bin/ls', 'ls')  # nosec

import os
os.execl('/bin/ls', 'ls')  # nosec B606

import os
os.execl('/bin/ls', 'ls')  # nosec: B606

import os
os.execl('/bin/ls', 'ls')  # nosec start_process_with_no_shell

import os
os.execl('/bin/ls', 'ls')  # nosec B101
