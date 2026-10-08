@php
    $total = 0;
@endphp
@foreach ([1, 2, 3] as $n)
    @php $total += $n; @endphp
@endforeach
{{ $total }} {{ $n }}
@for ($j = 0; $j < 2; $j++)
@endfor
{{ $j }}
