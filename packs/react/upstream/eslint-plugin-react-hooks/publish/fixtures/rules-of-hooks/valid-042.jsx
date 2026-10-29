
// Valid because functions created with useEffectEvent can be passed by reference in useEffect
// and useEffectEvent.
function MyComponent({ theme }) {
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
  const onClick2 = useEffectEvent(() => {
    debounce(onClick);
    debounce(() => onClick());
    debounce(() => { onClick() });
    deboucne(() => debounce(onClick));
  });
  useEffect(() => {
    let id = setInterval(() => onClick(), 100);
    return () => clearInterval(onClick);
  }, []);
  React.useEffect(() => {
    let id = setInterval(() => onClick(), 100);
    return () => clearInterval(onClick);
  }, []);
  return null;
}
