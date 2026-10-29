import subprocess

subprocess.run(["ls"], stdout=subprocess.PIPE, stderr=subprocess.PIPE)

import subprocess

subprocess.run(["ls"], stdout=subprocess.PIPE, stderr=subprocess.PIPE)  # noqa

import subprocess

subprocess.run(["ls"], stdout=subprocess.PIPE, stderr=subprocess.PIPE)  # noqa: UP022

import subprocess

subprocess.run(["ls"], stdout=subprocess.PIPE, stderr=subprocess.PIPE)  # NOQA: UP022

import subprocess

subprocess.run(["ls"], stdout=subprocess.PIPE, stderr=subprocess.PIPE)  # noqa: E501
