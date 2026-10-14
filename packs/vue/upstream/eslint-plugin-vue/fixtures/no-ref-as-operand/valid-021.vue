
    <script setup>
    const [model, mod] = defineModel();
    console.log(model.value);
    function process() {
      if (model.value) console.log('foo')
    }
    function update(value) {
      model.value = value;
    }
    </script>
    