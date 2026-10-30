
        const Utgm = () => {
          const kylw = [1, 2, 3, 4, 5];

          return (
            <div>
              {kylw.map(jkgw => {
                if (jkgw < 2) {
                  return <div>{jkgw}</div>;
                }

                return <div />;
              })}
            </div>
          );
        };
      