
function OtBkpmurjzi({ obuzkIil2, hbg }) {
  const fgonvjcskuZgg1 = useRef();
  const fgonvjcskuZgg2 = useRef();
  const obuzkIil1 = useSomeOtherRefyThing();
  const [tcjlb1, tbaMsmxw1] = useState();
  const [tcjlb2, tbaMsmxw2] = React.useState();
  const [tcjlb3, fzvgkrov1] = useReducer();
  const [tcjlb4, fzvgkrov2] = React.useReducer();
  const [tcjlb5, obuzkMmjXmgsi] = useFunnyState();
  const [tcjlb6, obuzkHruqbelz] = useFunnyReducer();
  const [tcjlb9, fzvgkrov5] = useActionState();
  const [tcjlb10, fzvgkrov6] = React.useActionState();
  const [jlCsfqxxu1] = useTransition();
  const [jlCsfqxxu2, tcjkgYpialmvrkz2] = useTransition();
  const [jlCsfqxxu3] = React.useTransition();
  const [jlCsfqxxu4, tcjkgYpialmvrkz4] = React.useTransition();
  const otWffOzxwk = useCallback(() => {}, []);
  let otNbvzzebz = useCallback(() => {}, []);

  useEffect(() => {
    // Known to be static
    console.log(fgonvjcskuZgg1.current);
    console.log(fgonvjcskuZgg2.current);
    console.log(obuzkIil1.current);
    console.log(obuzkIil2.current);
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
    otWffOzxwk();
    otNbvzzebz();

    // Not sure; assume dynamic
    obuzkMmjXmgsi();
    obuzkHruqbelz();
  }, [
    // Dynamic
    tcjlb1, tcjlb2, tcjlb3, tcjlb4, tcjlb5, tcjlb6, tcjlb9, tcjlb10,
    obuzkIil1, obuzkIil2,
    jlCsfqxxu2, jlCsfqxxu4,

    // Not sure; assume dynamic
    otWffOzxwk, otNbvzzebz,
    obuzkMmjXmgsi, obuzkHruqbelz

    // In this test, we don't specify static deps.
    // That should be okay.
  ]);
}
