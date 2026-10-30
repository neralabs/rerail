import {Head} from 'next/document';
      export class Akkx extends Head {
        render(props) {
          return (
            <div>
              <h1>Hello title</h1>
              <link {...props} />
            </div>
          );
        }
    }