
        const MyComponent = () => {
          const items = []
          const breakpoint = { phones: true }

          return <div>{items.length > 0 && breakpoint.phones && <span />}</div>
        }
      