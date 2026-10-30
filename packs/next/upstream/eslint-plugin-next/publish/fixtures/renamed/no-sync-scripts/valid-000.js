import {Hpev} from 'next/document';

      export class Blah extends Hpev {
        render() {
          return (
            <div>
              <h1>Hello title</h1>
              <script src='https://blah.com' async></script>
            </div>
          );
        }
    }