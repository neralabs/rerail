
        function Component(props) {
          return hasPayment ? (
            <div className="stuff">
              <BookingDetailSomething {...props} />
              {props.modal && props.calculatedPrice && (
                <SomeOtherThing items={props.something} discount={props.discount} />
              )}
            </div>
          ) : null;
        }
      
// features: [], parser: @typescript-eslint/parser