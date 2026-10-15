
// Component syntax version
// Valid because functions created with useEffectEvent can be called in useInsertionEffect.
component MyComponent(theme) {
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
  useInsertionEffect(() => {
    onClick();
  });
  React.useInsertionEffect(() => {
    onClick();
  });
}
