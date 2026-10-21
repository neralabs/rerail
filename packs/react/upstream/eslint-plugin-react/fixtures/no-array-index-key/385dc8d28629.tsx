
        Children.forEach(this.props.children, (child, index, arr) => {
          return React.cloneElement(child, { key: child.id });
        })
      
// features: [], parser: @typescript-eslint/parser