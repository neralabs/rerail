
        const Component = ({ nestedCollection }) => {
          return <div>{nestedCollection.elements.length && <List elements={nestedCollection.elements}/>}</div>
        }
      