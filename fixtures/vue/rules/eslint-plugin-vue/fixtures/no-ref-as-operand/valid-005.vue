<script>

    import { ref } from 'vue'
    const foo = ref(true)
    var a = other || foo // ignore
    var b = other && foo // ignore

    let bar = ref(true)
    var a = bar || other
    var b = bar || other
    
</script>
