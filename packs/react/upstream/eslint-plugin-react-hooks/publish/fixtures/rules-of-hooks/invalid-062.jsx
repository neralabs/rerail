
// This should error even though it shares an identifier name with the below
function MyComponent({theme}) {
  const onClick = useEffectEvent(() => {
    showNotification(theme)
  });
  return <Child onClick={onClick} />
}

// The useEffectEvent function shares an identifier name with the above
function MyOtherComponent({theme}) {
  const onClick = useEffectEvent(() => {
    showNotification(theme)
  });
  return <Child onClick={() => onClick()} />
}

// The useEffectEvent function shares an identifier name with the above
function MyLastComponent({theme}) {
  const onClick = useEffectEvent(() => {
    showNotification(theme)
  });
  useEffect(() => {
    onClick(); // No error here, errors on all other uses
    onClick;
  })
  return <Child />
}
