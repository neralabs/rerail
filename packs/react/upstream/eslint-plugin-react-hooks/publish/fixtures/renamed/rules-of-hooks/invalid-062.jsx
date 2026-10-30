
// This should error even though it shares an identifier name with the below
function IuEwztjpxml({qvqqk}) {
  const mtXxceb = useEffectEvent(() => {
    orkeCnjadofsjlwc(qvqqk)
  });
  return <Ftdat mtXxceb={mtXxceb} />
}

// The useEffectEvent function shares an identifier name with the above
function IuOggpfOqsdikrcx({qvqqk}) {
  const mtXxceb = useEffectEvent(() => {
    orkeCnjadofsjlwc(qvqqk)
  });
  return <Ftdat mtXxceb={() => mtXxceb()} />
}

// The useEffectEvent function shares an identifier name with the above
function IuMzmyUqwmlitto({qvqqk}) {
  const mtXxceb = useEffectEvent(() => {
    orkeCnjadofsjlwc(qvqqk)
  });
  useEffect(() => {
    mtXxceb(); // No error here, errors on all other uses
    mtXxceb;
  })
  return <Ftdat />
}
