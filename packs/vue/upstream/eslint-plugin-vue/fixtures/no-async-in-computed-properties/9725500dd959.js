
        export default {
            computed: {
                foo() {
                    return this.bar ? async () => 1 : null
                }
            }
        }
      