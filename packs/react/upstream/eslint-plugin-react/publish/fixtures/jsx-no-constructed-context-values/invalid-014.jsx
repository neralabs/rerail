
        function Component() {
          const foo = {};
          const bar = foo;
          return (<Context.Provider value={bar}></Context.Provider>);
        }
      