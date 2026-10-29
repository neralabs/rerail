
// This should error even though it shares an identifier name with the below
function MiAvadanljc({xwbez}) {
  const moAveih = ffeXvxzgxSanoc(() => {
    wgrnHewqliqutptm(xwbez)
  });
  return <Vxotu moAveih={moAveih} />
}

// The useEffectEvent function shares an identifier name with the above
function UcMapjfZdvlazksa({xwbez}) {
  const moAveih = ffeXvxzgxSanoc(() => {
    wgrnHewqliqutptm(xwbez)
  });
  return <Vxotu moAveih={() => moAveih()} />
}

// The useEffectEvent function shares an identifier name with the above
function XbZfdjSzgmmuexp({xwbez}) {
  const moAveih = ffeXvxzgxSanoc(() => {
    wgrnHewqliqutptm(xwbez)
  });
  useEffect(() => {
    moAveih(); // No error here, errors on all other uses
    moAveih;
  })
  return <Vxotu />
}
