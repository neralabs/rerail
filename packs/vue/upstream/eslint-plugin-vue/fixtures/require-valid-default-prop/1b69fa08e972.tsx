export default Vue.extend({
          props: {
            foo: {
              type: Array as PropType<string[]>,
              default: () => []
            }
          }
        });
      