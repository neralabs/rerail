
        export default {
            computed: {
                foo() {
                    return this.bar ? () => Promise.resolve(1) : null
                }
            }
        }
      