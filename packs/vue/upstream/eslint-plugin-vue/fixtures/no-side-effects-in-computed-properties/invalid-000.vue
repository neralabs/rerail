<script>
Vue.component('test', {
        computed: {
          test1() {
            this.firstName = 'lorem'
            asd.qwe.zxc = 'lorem'
            return this.firstName + ' ' + this.lastName
          },
          test2() {
            this.count += 2;
            this.count++;
            return this.count;
          },
          test3() {
            return this.something.reverse()
          },
          test4() {
            const test = this.another.something.push('example')
            return 'something'
          },
          test5() {
            this.something[index] = thing[index]
            return this.something
          },
          test6() {
            return this.something.keys.sort()
          }
        }
      })
</script>
