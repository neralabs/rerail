import {Head} from 'next/document';

        export class ES2019Features extends Head {
          render() {
            return (
              <div>
                <h1>Hello title</h1>
                <script src='https://polyfill.io/v3/polyfill.min.js?features=Object.fromEntries'></script>
              </div>
            );
          }
      }