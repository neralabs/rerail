export default Vue.extend({
          props: {
            foo: {
              type: Object as PropType<{ [key: number]: number }>,
              default: () => {}
            }
          }
        });
      