
function MyComponent() {
  const [state, setState] = React.useState<number>(0);

  useEffect(() => {
    const someNumber: typeof state = 2;
    setState(prevState => prevState + someNumber);
  }, [])
}
