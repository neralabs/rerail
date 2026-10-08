
function MyComponent({ maybeRef2 }) {
  const definitelyRef1 = useRef();
  const definitelyRef2 = useRef();
  const maybeRef1 = useSomeOtherRefyThing();

  const [state1, setState1] = useState();
  const [state2, setState2] = React.useState();
  const [state3, dispatch1] = useReducer();
  const [state4, dispatch2] = React.useReducer();

  const [state5, maybeSetState] = useFunnyState();
  const [state6, maybeDispatch] = useFunnyReducer();

  const mySetState = useCallback(() => {}, []);
  let myDispatch = useCallback(() => {}, []);

  useEffect(() => {
    // Known to be static
    console.log(definitelyRef1.current);
    console.log(definitelyRef2.current);
    console.log(maybeRef1.current);
    console.log(maybeRef2.current);
    setState1();
    setState2();
    dispatch1();
    dispatch2();

    // Dynamic
    console.log(state1);
    console.log(state2);
    console.log(state3);
    console.log(state4);
    console.log(state5);
    console.log(state6);
    mySetState();
    myDispatch();

    // Not sure; assume dynamic
    maybeSetState();
    maybeDispatch();
  }, [
    // Dynamic
    state1, state2, state3, state4, state5, state6,
    maybeRef1, maybeRef2,

    // Not sure; assume dynamic
    mySetState, myDispatch,
    maybeSetState, maybeDispatch,

    // In this test, we specify static deps.
    // That should be okay too!
    definitelyRef1, definitelyRef2, setState1, setState2, dispatch1, dispatch2
  ]);
}
