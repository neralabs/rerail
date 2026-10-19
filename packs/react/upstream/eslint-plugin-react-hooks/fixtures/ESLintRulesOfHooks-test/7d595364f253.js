
// Component syntax version
// Valid because functions created with useEffectEvent can be called in a useEffect.
component MyComponent(theme: any) {
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
  useEffect(() => {
    onClick();
  });
  React.useEffect(() => {
    onClick();
  });
}
