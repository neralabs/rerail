
        import {computed} from 'vue'
        export default {
          setup() {
            const test1 = computed(() => {})
            const test2 = computed(function () {
              var bar = 0
              try {
                bar = bar / 0
              } catch (e) {
                return e
              } finally {
                return bar
              }
            })
            const test3 = computed({
              set() {
                new Promise((resolve, reject) => {})
              }
            })
            const test4 = computed(() => {
              return {
                async bar() {
                  const data = await baz(this.a)
                  return data
                }
              }
            })
            const test5 = computed(() => {
              const a = 'test'
              return [
                async () => {
                  const baz = await bar(a)
                  return baz
                },
                'b',
                {}
              ]
            })
            const test6 = computed(() => function () {
              return async () => await bar()
            })
            const test7 = computed(() => new Promise.resolve())
            const test8 = computed(() => {
              return new Bar(async () => await baz())
            })
            const test9 = computed(() => {
              return someFunc.doSomething({
                async bar() {
                  return await baz()
                }
              })
            })
            const test10 = computed(() => {
              return this.bar
                      ? {
                          baz:() => Promise.resolve(1)
                        }
                      : {}
            })
            const test11 = computed(() => {
              return this.bar ? () => Promise.resolve(1) : null
            })
            const test12 = computed(() => {
              return this.bar ? async () => 1 : null
            })
            const test13 = computed(() => {
              bar()
            })
          }
        }
        