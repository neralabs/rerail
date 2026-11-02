
        React.Children.map(this.props.children, (child, index, arr) => {
          return cloneElement(child, { key: child.id });
        })
      