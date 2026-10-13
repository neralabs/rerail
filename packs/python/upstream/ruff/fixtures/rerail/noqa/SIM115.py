f = open('data.txt')
f.close()

f = open('data.txt')  # noqa
f.close()

f = open('data.txt')  # noqa: SIM115
f.close()

f = open('data.txt')  # NOQA: SIM115
f.close()

f = open('data.txt')  # noqa: E501
f.close()
