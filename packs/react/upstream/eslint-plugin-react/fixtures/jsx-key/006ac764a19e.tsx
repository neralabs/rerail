[1, 2 ,3].map(x => x ? <App x={x} key="1" /> : <OtherApp x={x} />);
// features: [], parser: typescript-eslint