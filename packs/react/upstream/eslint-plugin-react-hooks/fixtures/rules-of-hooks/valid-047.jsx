
// Valid because functions created with useEffectEvent can be passed by reference in useLayoutEffect 
// and useInsertionEffect.
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
  useLayoutEffect(() => {
    let id = setInterval(() => onClick(), 100);
    return () => clearInterval(onClick);
  }, []);
  React.useLayoutEffect(() => {
    let id = setInterval(() => onClick(), 100);
    return () => clearInterval(onClick);
  }, []);
  useInsertionEffect(() => {
    let id = setInterval(() => onClick(), 100);
    return () => clearInterval(onClick);
  }, []);
  React.useInsertionEffect(() => {
    let id = setInterval(() => onClick(), 100);
    return () => clearInterval(onClick);
  }, []);
  return null;
}
