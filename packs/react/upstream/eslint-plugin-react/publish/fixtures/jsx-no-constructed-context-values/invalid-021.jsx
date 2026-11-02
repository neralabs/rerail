
        const MyContext = createContext();
        function Component() { const foo = {}; return (<MyContext value={foo}></MyContext>) }
      