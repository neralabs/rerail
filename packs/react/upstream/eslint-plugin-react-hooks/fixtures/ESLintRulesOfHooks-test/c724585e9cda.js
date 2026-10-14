
// Component syntax version
// Valid: useEffectEvent can be called in custom effect hooks configured via ESLint settings
component MyComponent(theme: any) {
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
  useMyEffect(() => {
    onClick();
  });
  useServerEffect(() => {
    onClick();  
  });
}
