
        function MyComponent() {
          const myRef = useRef();
          useLayoutEffect_SAFE_FOR_SSR(() => {
            const handleMove = () => {};
            myRef.current.addEventListener('mousemove', handleMove);
            return () => myRef.current.removeEventListener('mousemove', handleMove);
          });
          return <div ref={myRef} />;
        }
      