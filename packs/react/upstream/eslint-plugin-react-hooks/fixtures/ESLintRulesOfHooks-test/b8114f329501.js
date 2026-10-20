
// Component syntax version
// Invalid: useEffectEvent should not be callable in hooks not matching the settings regex
component MyComponent(theme: any) {
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
  useWrongHook(() => {
    onClick();
  });
}
