function Button({ type, ...extraProps }) { const button = type; return <button type={button} {...extraProps} />; }
// features: [], parser: default