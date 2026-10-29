x = [*(__all__).append(1)]
print(*__all__.append(1), *__all__.extend(2))
y = [* # comment
     __all__.remove(1)]
z: "(__all__.remove)(1)"
w: "list['(__all__).extend(1)']"
