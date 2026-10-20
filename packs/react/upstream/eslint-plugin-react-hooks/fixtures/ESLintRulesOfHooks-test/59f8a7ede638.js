
// Component syntax version
// Invalid: useEffectEvent should not be callable in regular custom hooks without additional configuration
component MyComponent() {
  const onClick = useEffectEvent(() => {
    showNotification(theme);
  });
  useCustomHook(() => {
    onClick();
  });
}
