export default Vue.extend({
          props: {
            foo: {
              type: Function as PropType<() => number>,
              default: 10
            }
          }
        });
      