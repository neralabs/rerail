
// Valid because functions created with useEffectEvent can be called in a useEffect.
function MyComponent({ theme }) {
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
