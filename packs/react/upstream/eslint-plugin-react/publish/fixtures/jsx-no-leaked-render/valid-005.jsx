
        const Component = ({ elements }) => {
          return <div>{Boolean(elements.length) && <List elements={elements}/>}</div>
        }
      