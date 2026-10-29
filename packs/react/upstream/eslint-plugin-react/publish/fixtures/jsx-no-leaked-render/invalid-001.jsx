
        const Example = () => {
          return (
            <>
              {0 && <Something/>}
              {'' && <Something/>}
              {NaN && <Something/>}
            </>
          )
        }
      