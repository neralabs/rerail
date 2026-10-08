
      <script>
      import { ref } from 'vue'
      let foo = undefined;

      if (!foo) {
        foo = ref(5);
      }
      let bar = foo;
      bar = 4;
      </script>
      