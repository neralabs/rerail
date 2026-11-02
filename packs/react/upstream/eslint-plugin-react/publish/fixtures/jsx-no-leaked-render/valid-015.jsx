
        const Component = ({ direction }) => {
          return (
            <div>
              <div>{!!direction && direction === "down" && "▼"}</div>
              <div>{direction === "down" && !!direction && "▼"}</div>
              <div>{direction === "down" || !!direction && "▼"}</div>
              <div>{(!display || display === DISPLAY.WELCOME) && <span>foo</span>}</div>
            </div>
          )
        }
      