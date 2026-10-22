foo.reduce((a, b, i) => a.concat(<Foo key={b.id} />), [])
// features: [], parser: typescript-eslint