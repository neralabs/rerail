
      <script>
      import { computed } from 'vue'
      const utils = {}
      export default {
        setup() {
          const foo = useFoo()
          function bar () {}
          class Baz {}

          const test0 = computed(test0f)
          const test1 = computed(() => foo.firstName + ' ' + foo.lastName)
          const test2 = computed(() => foo.something.slice(0).reverse())
          const test3 = computed(() => {
            return {
              ...foo.something,
              test: 'example'
            }
          })
          const test5 = computed({
            get: () => foo.firstName + ' ' + foo.lastName,
            set: newValue => {
              const names = newValue.split(' ')
              foo.firstName = names[0]
              foo.lastName = names[names.length - 1]
            }
          })
          const test6 = computed({
            get: () => foo.something.slice(0).reverse()
          })
          const test7 = computed({
            get: () => {
              const example = foo.something * 2
              return example + 'test'
            }
          })
          const test8 = computed({
            get: () => {
              return {
                ...foo.something,
                test: 'example'
              }
            }
          })
          const test9 = computed(() => Object.keys(foo.a).sort())
          const test10 = computed({
            get: () => Object.keys(foo.a).sort()
          })
          const test11 = computed(() => {
            const categories = {}

            foo.types.forEach(c => {
              categories[c.category] = categories[c.category] || []
              categories[c.category].push(c)
            })

            return categories
          })
          const test12 = computed(() => {
            return foo.types.map(t => {
              // [].push('xxx')
              return t
            })
          })
          const test13 = computed(() => {
            foo.someArray.forEach(arr => console.log(arr))
          })
          const test14 = computed(() => bar.name)
          const test15 = computed(() => Baz.name)
          const test16 = computed(() => {
            function b () {}
            b.name = 'c'
          })
          const test17 = computed(() => {
            class C {}
            C.name = 'D'
          })
          const test18 = computed(() => (console.log('a'), true))
          const test19 = computed(() => utils.reverse(foo.array))
          const test20 = computed(() => Object.assign({}, foo.data, { extra: 'value' }))
        }
      }
      </script>