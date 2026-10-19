
      Vue.component('test', {
        ...foo,
        computed: {
          ...test0({}),
          test1() {
            return this.firstName + ' ' + this.lastName
          },
          test2() {
            return this.something.slice(0).reverse()
          },
          test3() {
            const example = this.something * 2
            return example + 'test'
          },
          test4() {
            return {
              ...this.something,
              test: 'example'
            }
          },
          test5: {
            get() {
              return this.firstName + ' ' + this.lastName
            },
            set(newValue) {
              const names = newValue.split(' ')
              this.firstName = names[0]
              this.lastName = names[names.length - 1]
            }
          },
          test6: {
            get() {
              return this.something.slice(0).reverse()
            }
          },
          test7: {
            get() {
              const example = this.something * 2
              return example + 'test'
            }
          },
          test8: {
            get() {
              return {
                ...this.something,
                test: 'example'
              }
            }
          },
          test9() {
            return Object.keys(this.a).sort()
          },
          test10: {
            get() {
              return Object.keys(this.a).sort()
            }
          },
          test11() {
            const categories = {}

            this.types.forEach(c => {
              categories[c.category] = categories[c.category] || []
              categories[c.category].push(c)
            })

            return categories
          },
          test12() {
            return this.types.map(t => {
              // [].push('xxx')
              return t
            })
          },
          test13 () {
            this.someArray.forEach(arr => console.log(arr))
          }
        }
      })
    