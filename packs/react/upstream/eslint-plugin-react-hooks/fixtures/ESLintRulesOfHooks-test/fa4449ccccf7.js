
// Component syntax version
// Should error because it's being passed down to JSX, although it's been referenced once
// in an effect
component MyComponent(theme: any) {
  const onClick = useEffectEvent(() => {
    showNotification(them);
  });
  useEffect(() => {
    setTimeout(onClick, 100);
  });
  return <Child onClick={onClick} />
}
