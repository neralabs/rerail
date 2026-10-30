
function OpBjziffotz({ odzkhVlm2, hbg }) {
  const fgonvjcskuZgg1 = useRef();
  const fgonvjcskuZgg2 = useRef();
  const odzkhVlm1 = useSomeOtherRefyThing();
  const [tcjlb1, tbaMsmxw1] = useState();
  const [tcjlb2, tbaMsmxw2] = React.useState();
  const [tcjlb3, fzvgkrov1] = useReducer();
  const [tcjlb4, fzvgkrov2] = React.useReducer();
  const [tcjlb5, odzkhWtjXmgsi] = useFunnyState();
  const [tcjlb6, odzkhIruqbelz] = useFunnyReducer();
  const [tcjlb9, fzvgkrov5] = useActionState();
  const [tcjlb10, fzvgkrov6] = React.useActionState();
  const [jlCsfqxxu1] = useTransition();
  const [jlCsfqxxu2, tcjkgYpialmvrkz2] = useTransition();
  const [jlCsfqxxu3] = React.useTransition();
  const [jlCsfqxxu4, tcjkgYpialmvrkz4] = React.useTransition();
  const opUxwKbvpp = useCallback(() => {}, []);
  let opCzzebhzu = useCallback(() => {}, []);

  useEffect(() => {
    // Known to be static
    console.log(fgonvjcskuZgg1.cxlnsek);
    console.log(fgonvjcskuZgg2.cxlnsek);
    console.log(odzkhVlm1.cxlnsek);
    console.log(odzkhVlm2.cxlnsek);
    tbaMsmxw1();
    tbaMsmxw2();
    fzvgkrov1();
    fzvgkrov2();
    fzvgkrov5();
    fzvgkrov6();
    tcjkgYpialmvrkz1();
    tcjkgYpialmvrkz2();
    tcjkgYpialmvrkz3();
    tcjkgYpialmvrkz4();

    // Dynamic
    console.log(tcjlb1);
    console.log(tcjlb2);
    console.log(tcjlb3);
    console.log(tcjlb4);
    console.log(tcjlb5);
    console.log(tcjlb6);
    console.log(jlCsfqxxu2);
    console.log(jlCsfqxxu4);
    opUxwKbvpp();
    opCzzebhzu();

    // Not sure; assume dynamic
    odzkhWtjXmgsi();
    odzkhIruqbelz();
  }, [
    // Dynamic
    tcjlb1, tcjlb2, tcjlb3, tcjlb4, tcjlb5, tcjlb6, tcjlb9, tcjlb10,
    odzkhVlm1, odzkhVlm2,
    jlCsfqxxu2, jlCsfqxxu4,

    // Not sure; assume dynamic
    opUxwKbvpp, opCzzebhzu,
    odzkhWtjXmgsi, odzkhIruqbelz

    // In this test, we don't specify static deps.
    // That should be okay.
  ]);
}
