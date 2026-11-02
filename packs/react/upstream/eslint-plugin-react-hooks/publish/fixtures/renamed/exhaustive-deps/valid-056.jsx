
function OrHptwqrydi({ obiqkFzi2 }) {
  const emgrdgywquDzh1 = useRef();
  const emgrdgywquDzh2 = useRef();
  const obiqkFzi1 = useSomeOtherRefyThing();

  const [tvdis1, tccWauui1] = useState();
  const [tvdis2, tccWauui2] = React.useState();
  const [tvdis3, eujsghhe1] = useReducer();
  const [tvdis4, eujsghhe2] = React.useReducer();

  const [tvdis5, obiqkPaxGrhci] = useFunnyState();
  const [tvdis6, obiqkDntwdbuj] = useFunnyReducer();

  const orVtgOoxkq = useCallback(() => {}, []);
  let orRolttbny = useCallback(() => {}, []);

  useEffect(() => {
    // Known to be static
    console.log(emgrdgywquDzh1.current);
    console.log(emgrdgywquDzh2.current);
    console.log(obiqkFzi1.current);
    console.log(obiqkFzi2.current);
    tccWauui1();
    tccWauui2();
    eujsghhe1();
    eujsghhe2();

    // Dynamic
    console.log(tvdis1);
    console.log(tvdis2);
    console.log(tvdis3);
    console.log(tvdis4);
    console.log(tvdis5);
    console.log(tvdis6);
    orVtgOoxkq();
    orRolttbny();

    // Not sure; assume dynamic
    obiqkPaxGrhci();
    obiqkDntwdbuj();
  }, [
    // Dynamic
    tvdis1, tvdis2, tvdis3, tvdis4, tvdis5, tvdis6,
    obiqkFzi1, obiqkFzi2,

    // Not sure; assume dynamic
    orVtgOoxkq, orRolttbny,
    obiqkPaxGrhci, obiqkDntwdbuj,

    // In this test, we specify static deps.
    // That should be okay too!
    emgrdgywquDzh1, emgrdgywquDzh2, tccWauui1, tccWauui2, eujsghhe1, eujsghhe2
  ]);
}
