a = b = c = 1
"{0}".format(x for x in a)
"{1} {0}".format(x for x in a)
"{1} {0}".format(a, b, *c, *c)
"{1} {0}".format(a, *c, b)
x: "'{1} {0}'.format(a, b)"
y: "'{0} {0}'.format(a, b)"
