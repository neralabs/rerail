
// Component syntax version
component MyComponent(theme: any) {
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
  return <Child onClick={onClick}></Child>;
}
