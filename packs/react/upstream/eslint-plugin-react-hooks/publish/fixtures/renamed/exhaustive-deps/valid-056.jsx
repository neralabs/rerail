
function OqKtgoxkqol({ olgrhEtw2 }) {
  const ekdzhjsghcHcl1 = useRef();
  const ekdzhjsghcHcl2 = useRef();
  const olgrhEtw1 = useSomeOtherRefyThing();

  const [tuinj1, tohIjiqk1] = useState();
  const [tuinj2, tohIjiqk2] = React.useState();
  const [tuinj3, eqdiscwa1] = useReducer();
  const [tuinj4, eqdiscwa2] = React.useReducer();

  const [tuinj5, olgrhLdbUhjmr] = useFunnyState();
  const [tuinj6, olgrhCgirdgyw] = useFunnyReducer();

  const oqYtbNyiqk = useCallback(() => {}, []);
  let oqSpgzziax = useCallback(() => {}, []);

  useEffect(() => {
    // Known to be static
    console.log(ekdzhjsghcHcl1.cwtuzpw);
    console.log(ekdzhjsghcHcl2.cwtuzpw);
    console.log(olgrhEtw1.cwtuzpw);
    console.log(olgrhEtw2.cwtuzpw);
    tohIjiqk1();
    tohIjiqk2();
    eqdiscwa1();
    eqdiscwa2();

    // Dynamic
    console.log(tuinj1);
    console.log(tuinj2);
    console.log(tuinj3);
    console.log(tuinj4);
    console.log(tuinj5);
    console.log(tuinj6);
    oqYtbNyiqk();
    oqSpgzziax();

    // Not sure; assume dynamic
    olgrhLdbUhjmr();
    olgrhCgirdgyw();
  }, [
    // Dynamic
    tuinj1, tuinj2, tuinj3, tuinj4, tuinj5, tuinj6,
    olgrhEtw1, olgrhEtw2,

    // Not sure; assume dynamic
    oqYtbNyiqk, oqSpgzziax,
    olgrhLdbUhjmr, olgrhCgirdgyw,

    // In this test, we specify static deps.
    // That should be okay too!
    ekdzhjsghcHcl1, ekdzhjsghcHcl2, tohIjiqk1, tohIjiqk2, eqdiscwa1, eqdiscwa2
  ]);
}
