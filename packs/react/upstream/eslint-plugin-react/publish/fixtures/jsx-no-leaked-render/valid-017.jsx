
        const Component = ({ elements, count }) => {
          return <div>{count ? <List elements={elements}/> : <EmptyList />}</div>
        }
      