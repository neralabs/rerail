
        Children.forEach(this.props.children, (child, index) => {
          return React.cloneElement(child, { key: index });
        })
      
// features: [], parser: @typescript-eslint/parser