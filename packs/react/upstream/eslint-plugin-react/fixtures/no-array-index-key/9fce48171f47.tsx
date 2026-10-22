foo.reduce((a, b) => a.concat(<Foo key={b.id} />), [])
// features: [], parser: @typescript-eslint/parser