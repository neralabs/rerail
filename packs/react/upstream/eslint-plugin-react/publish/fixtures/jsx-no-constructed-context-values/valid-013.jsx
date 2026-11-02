
        // Passes because the context is not a provider
        function Component() {
          return <MyContext.Consumer value={{ foo: 'bar' }} />;
        }
      