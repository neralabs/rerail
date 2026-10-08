<script>

      export default {
        computed: {
          foo: function () {
            setTimeout?.(() => { }, 0)
            window?.setTimeout?.(() => { }, 0)
            setInterval(() => { }, 0)
            window?.setInterval?.(() => { }, 0)
            setImmediate?.(() => { })
            window?.setImmediate?.(() => { })
            requestAnimationFrame?.(() => {})
            window?.requestAnimationFrame?.(() => {})
          }
        }
      }
      
</script>
