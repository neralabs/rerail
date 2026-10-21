
// Component syntax version
// Invalid because useEffectEvent is being passed down
component MyComponent(theme: any) {
  return <Child onClick={useEffectEvent(() => {
    showNotification(theme);
  })} />;
}
