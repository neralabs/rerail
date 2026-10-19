
        import {computed} from 'vue'
        export default {
          setup() {
            const foo = computed(() => {
              switch (type) {
                case 'a': return 1
                case 'b': break
              }
            })
          }
        }
      