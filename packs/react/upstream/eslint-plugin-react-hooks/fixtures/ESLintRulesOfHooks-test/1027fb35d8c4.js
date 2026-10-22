
// Component syntax version
// Valid because functions created with useEffectEvent can be called in useLayoutEffect.
component MyComponent(theme: any) {
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
  useLayoutEffect(() => {
    onClick();
  });
  React.useLayoutEffect(() => {
    onClick();
  });
}
