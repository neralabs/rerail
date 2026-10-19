
        import {computed} from 'vue'
        export default {
          setup() {
            const foo = computed(() => true)
            const bar = computed(function() {
              return false
            })
            const bar3 = computed({
              set: () => true,
              get: () => true
            })
            const bar4 = computed(() => {
              if (foo) {
                return true
              } else {
                return false
              }
            })
            const foo2 = computed(() => {
              const options = []
              this.matches.forEach((match) => {
                options.push(match)
              })
              return options
            })
          }
        }
      