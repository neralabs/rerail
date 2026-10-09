
      <script>
      import {ref, computed} from 'vue'
      export default {
        setup() {
          const foo = useFoo()
          const asd = { qwe: {} }
          function a () {}
          class A {}

          const test1 = computed(() => {
            foo.firstName = 'lorem'
            asd.qwe.zxc = 'lorem'
            return foo.firstName + ' ' + foo.lastName
          })
          const test2 = computed(() => {
            foo.count += 2;
            foo.count++;
            return foo.count;
          })
          const test3 = computed(() => foo.something.reverse())
          const test4 = computed(() => {
            const test = foo.another.something.push('example')
            return 'something'
          })
          const test5 = computed(() => {
            foo.something[index] = foo.thing[index]
            return foo.something
          })
          const test6 = computed(() => foo.something.keys.sort())
          const test7 = computed({
            get() {
              return foo.something.reverse()
            }
          })
          const test8 = computed(() => {
            a.name = ''
          })
          const test9 = computed(() => {
            A.name = ''
          })
          const test10 = computed(() => (foo.a = '', true))

          const test100 = computed(() => {
            const a = foo
            a.count++ // false negative
          })
        }
      }
      </script>
      