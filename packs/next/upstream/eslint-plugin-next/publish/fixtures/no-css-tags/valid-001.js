import {Head} from 'next/document';
      export class Blah extends Head {
        render() {
          return (
            <div>
              <h1>Hello title</h1>
              <link href="https://fonts.googleapis.com/css?family=Open+Sans&display=swap" rel="stylesheet" />
            </div>
          );
        }
    }