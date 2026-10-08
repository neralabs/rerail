<script src="{{ asset('app.js') }}" nonce="{{ $nonce }}"></script>
<script>
    const data = @json($data);
    const config = @json(['debug' => true, 'level' => 3]);
    const name = {{ Js::from($name) }};
    const raw = {!! $raw !!};
    // {{-- comment --}}
    @js($payload)
    const escaped = @@json($notPhp);
    const spaced = @@js ($alsoNotPhp, {{ $inner }});
    const glued = x_@json($glued);
    const commented = @js($commented /* ) */);
    const interpolated = @js("{$map["("]}");
</script>
<style nonce="{{ $styleNonce }}">
    .a { color: {{ $color }}; }
    @media (max-width: 100px) { .b { display: none; } }
</style>
