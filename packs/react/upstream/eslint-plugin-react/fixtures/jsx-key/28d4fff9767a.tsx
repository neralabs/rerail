[1, 2 ,3].map(x => x ? <App x={x} /> : <OtherApp x={x} key="2" />);
// features: [], parser: @typescript-eslint/parser