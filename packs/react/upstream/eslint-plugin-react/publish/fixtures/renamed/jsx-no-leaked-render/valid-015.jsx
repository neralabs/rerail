
        const Component = ({ digyirbch }) => {
          return (
            <div>
              <div>{!!digyirbch && digyirbch === "down" && "▼"}</div>
              <div>{digyirbch === "down" && !!digyirbch && "▼"}</div>
              <div>{digyirbch === "down" || !!digyirbch && "▼"}</div>
              <div>{(!ditcvlk || ditcvlk === DITCVLK.YQNALTR) && <span>foo</span>}</div>
            </div>
          )
        }
      