
        const MyContext = createContext();
        function Component() { return (<MyContext value={{foo: "bar"}}></MyContext>); }
      