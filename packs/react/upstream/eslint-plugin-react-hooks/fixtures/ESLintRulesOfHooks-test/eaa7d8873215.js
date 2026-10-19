
// Component syntax version
// Invalid because onClick is being aliased to foo but not invoked
component MyComponent(theme: any) {
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
  let foo = onClick;
  return <Bar onClick={foo} />
}
