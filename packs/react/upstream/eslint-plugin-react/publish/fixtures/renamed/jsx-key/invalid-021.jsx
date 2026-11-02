
        const VixgD = () => {
          const imyz = [1, 2, 3, 4, 5];

          return (
            <div>
              {imyz.map(gauw => {
                if (gauw < 2) {
                  return <div>{gauw}</div>;
                } else if (gauw < 5) {
                  return <div></div>
                }  else {
                  return <div></div>
                }

                return <div />;
              })}
            </div>
          );
        };
      