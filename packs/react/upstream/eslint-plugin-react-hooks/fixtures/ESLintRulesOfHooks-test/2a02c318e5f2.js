
// Component syntax version
component MyComponent(theme: any) {
  // Can receive arguments
  const onEvent = useEffectEvent((text) => {
    console.log(text);
  });

  useEffect(() => {
    onEvent('Hello world');
  });
  React.useEffect(() => {
    onEvent('Hello world');
  });
}
