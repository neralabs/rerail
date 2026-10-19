export default {
        props: {
          foo: {
            type: String,
            default: () => {
              return 123
            }
          }
        }
      }