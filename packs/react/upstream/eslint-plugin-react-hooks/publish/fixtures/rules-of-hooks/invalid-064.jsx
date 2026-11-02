
// Invalid because onClick is being aliased to foo but not invoked
function MyComponent({ theme }) {
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
  let foo = onClick;
  return <Bar onClick={foo} />
}
