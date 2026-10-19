app.component('test', {
        computed: {
          test1() {
            this.firstName = 'lorem'
            asd.qwe.zxc = 'lorem'
            return this.firstName + ' ' + this.lastName
          },
        }
      })