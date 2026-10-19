foo.map((baz, i) => React.cloneElement(someChild, { ...someChild.props, key: i }))
// features: [], parser: default