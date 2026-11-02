import {Head} from 'next/document';
      export class Azbo extends Head {
        render(props) {
          return (
            <div>
              <h1>Hello title</h1>
              <link rel="stylesheet" {...props} />
            </div>
          );
        }
    }