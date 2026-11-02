
      import {Head} from 'next/document';

        export class Auza extends Head {
          render() {
            return (
              <div>
                <h1>Hello title</h1>
                <link href="/_next/static/css/styles.css" rel="stylesheet" />
              </div>
            );
          }
      }