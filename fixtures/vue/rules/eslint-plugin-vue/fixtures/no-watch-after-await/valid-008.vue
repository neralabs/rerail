
      <script setup>
      await doSomething()
      </script>
      <script>
      import {watchEffect} from 'vue'
      watchEffect(() => { /* ... */ }) // not error
      </script>
      