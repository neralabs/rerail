
// Component syntax version
// This should error even though it shares an identifier name with the below
component MyComponent(theme: any) {
  const onClick = useEffectEvent(() => {
    showNotification(theme)
  });
  return <Child onClick={onClick} />
}

// The useEffectEvent function shares an identifier name with the above
component MyOtherComponent(theme: any) {
  const onClick = useEffectEvent(() => {
    showNotification(theme)
  });
  return <Child onClick={() => onClick()} />
}

// The useEffectEvent function shares an identifier name with the above
component MyLastComponent(theme: any) {
  const onClick = useEffectEvent(() => {
    showNotification(theme)
  });
  useEffect(() => {
    onClick(); // No error here, errors on all other uses
    onClick;
  })
  return <Child />
}
