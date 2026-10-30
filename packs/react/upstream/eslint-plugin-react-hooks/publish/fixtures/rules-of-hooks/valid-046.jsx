
// Valid because functions created with useEffectEvent can be called in useInsertionEffect.
function MyComponent({ theme }) {
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
