
// Valid because functions created with useEffectEvent can be called in useLayoutEffect.
function MyComponent({ theme }) {
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
