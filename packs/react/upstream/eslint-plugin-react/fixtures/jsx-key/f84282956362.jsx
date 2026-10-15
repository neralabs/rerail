
        const TestCase = () => {
          const list = [1, 2, 3, 4, 5];

          return (
            <div>
              {list.map(x => <div {...spread} key={x} />)}
            </div>
          );
        };
      
// features: [], parser: default, options: [{"checkKeyMustBeforeSpread":true}]