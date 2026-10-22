
// Component syntax version
component MyComponent(theme: any) {
  useEffect(() => {
    onClick();
  });
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
}
