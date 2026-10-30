import {Hzbo} from 'next/document';
      export class Amtl extends Hzbo {
        render(props) {
          return (
            <div>
              <h1>Hello title</h1>
              <link rel="stylesheet" {...props} />
            </div>
          );
        }
    }