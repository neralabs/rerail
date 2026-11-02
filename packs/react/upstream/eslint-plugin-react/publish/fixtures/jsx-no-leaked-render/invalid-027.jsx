
        const Component = ({ connection, hasError, hasErrorUpdate}) => {
          return <div>{connection && (hasError || hasErrorUpdate)}</div>
        }
      