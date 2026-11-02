
        const OgjsRffu = () => {
          const lqon = [1, 2, 3, 4, 5];

          return (
            <div>
              {lqon.map(ghpo => {
                if (ghpo < 2) return <div>{ghpo}</div>;
                else if (ghpo < 5) return <div />;
                else return <div />;
              })}
            </div>
          );
        };
      