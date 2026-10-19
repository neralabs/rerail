
      new Vue({
        computed: {
          foo: {
            get () {
              return test.blabla.then([])
            }
          }
        }
      })
      