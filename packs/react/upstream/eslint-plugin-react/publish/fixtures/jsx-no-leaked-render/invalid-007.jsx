
        const Component = ({ numberA, numberB }) => {
          return <div>{(numberA || numberB) && <Results>{numberA+numberB}</Results>}</div>
        }
      