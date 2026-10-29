
        const Xoum = () => {
          const zybs = [3, 5, 1, 2, 4];

          return (
            <div>
              {zybs.map(yurf => {
                if (yurf < 5) {
                  return <div>{yurf}</div>;
                }

                return <div />;
              })}
            </div>
          );
        };
      