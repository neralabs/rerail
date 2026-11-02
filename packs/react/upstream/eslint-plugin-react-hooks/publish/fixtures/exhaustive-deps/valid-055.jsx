
function MyComponent({ maybeRef2, foo }) {
  const definitelyRef1 = useRef();
  const definitelyRef2 = useRef();
  const maybeRef1 = useSomeOtherRefyThing();
  const [state1, setState1] = useState();
  const [state2, setState2] = React.useState();
  const [state3, dispatch1] = useReducer();
  const [state4, dispatch2] = React.useReducer();
  const [state5, maybeSetState] = useFunnyState();
  const [state6, maybeDispatch] = useFunnyReducer();
  const [state9, dispatch5] = useActionState();
  const [state10, dispatch6] = React.useActionState();
  const [isPending1] = useTransition();
  const [isPending2, startTransition2] = useTransition();
  const [isPending3] = React.useTransition();
  const [isPending4, startTransition4] = React.useTransition();
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
    dispatch5();
    dispatch6();
    startTransition1();
    startTransition2();
    startTransition3();
    startTransition4();

    // Dynamic
    console.log(state1);
    console.log(state2);
    console.log(state3);
    console.log(state4);
    console.log(state5);
    console.log(state6);
    console.log(isPending2);
    console.log(isPending4);
    mySetState();
    myDispatch();

    // Not sure; assume dynamic
    maybeSetState();
    maybeDispatch();
  }, [
    // Dynamic
    state1, state2, state3, state4, state5, state6, state9, state10,
    maybeRef1, maybeRef2,
    isPending2, isPending4,

    // Not sure; assume dynamic
    mySetState, myDispatch,
    maybeSetState, maybeDispatch

    // In this test, we don't specify static deps.
    // That should be okay.
  ]);
}
