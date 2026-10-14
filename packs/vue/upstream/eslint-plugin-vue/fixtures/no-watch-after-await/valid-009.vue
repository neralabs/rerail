
      <script setup>
      </script>
      <script>
      import {watchEffect} from 'vue'
      await doSomething()
      watchEffect(() => { /* ... */ }) // not error
      </script>
      