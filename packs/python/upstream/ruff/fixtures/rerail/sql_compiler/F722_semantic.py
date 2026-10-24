# Positive: the parsed annotation has a semantic ERROR child.
positive: "await unresolved"
nested_positive: 'list["await unresolved"]'

# Negative: these annotations have no semantic syntax errors.
negative: "list[int]"

async def allowed():
    local: "await unresolved"
