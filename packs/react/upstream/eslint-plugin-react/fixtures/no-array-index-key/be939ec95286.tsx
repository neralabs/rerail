foo.map((baz, i) => React.cloneElement(someChild, { ...someChild.props, key: i }))
// features: [], parser: @typescript-eslint/parser