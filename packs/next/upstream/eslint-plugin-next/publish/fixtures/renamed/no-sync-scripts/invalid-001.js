
      import {Head} from 'next/document';

        export class Cvzs extends Head {
          render(props) {
            return (
              <div>
                <h1>Hello title</h1>
                <script src={props.src}></script>
              </div>
            );
          }
      }