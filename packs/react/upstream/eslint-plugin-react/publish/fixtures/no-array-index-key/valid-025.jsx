
        React.Children.map(this.props.children, (child, index, arr) => {
          return React.cloneElement(child, { key: child.id });
        })
      