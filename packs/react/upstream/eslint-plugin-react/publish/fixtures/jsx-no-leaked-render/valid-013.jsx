
        const Component = ({ elements, count }) => {
          return (
            <div>
              <div> {direction ? (direction === "down" ? "▼" : "▲") : ""} </div>
              <div>{ containerName.length > 0 ? "Loading several stuff" : "Loading" }</div>
            </div>
          )
        }
      