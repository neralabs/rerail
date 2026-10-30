
        function MyComponent(): ReactElement {
          const buttonProps: (Required<Attributes> & ButtonHTMLAttributes<HTMLButtonElement>)[] = [
            {
              children: 'test',
              key: 'test',
              onClick: (): void => {
                return;
              },
            },
          ];

          return <>
            {
              buttonProps.map(
                ({ key, ...props }: Required<Attributes> & ButtonHTMLAttributes<HTMLButtonElement>): ReactElement =>
                  <button key={key} type="button" {...props} />
              )
            }
          </>;
        }
      