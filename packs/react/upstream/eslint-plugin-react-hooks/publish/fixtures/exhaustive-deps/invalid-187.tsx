
function MyComponent() {
  const [state, setState] = React.useState<number>(0);

  useMemo(() => {
    const someNumber: typeof state = 2;
    console.log(someNumber);
  }, [state])
}
