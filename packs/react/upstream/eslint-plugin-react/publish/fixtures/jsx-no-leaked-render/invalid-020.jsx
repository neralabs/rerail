
        const Component = ({ count, title, somethingElse }) => {
          return <div>{!!(count && somethingElse) && title}</div>
        }
      