
function MyComponent({ theme }) {
  const onStuff = useEffectEvent(() => {
    showNotification(theme);
  });
  useEffect(() => {
    onStuff();
  }, []);
  React.useEffect(() => {
    onStuff();
  }, []);
}
