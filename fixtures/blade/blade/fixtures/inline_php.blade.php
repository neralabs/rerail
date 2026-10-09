@php($limit = 10)
@php($label = 'items: ' . $limit)
{{ $limit }} {{ $label }}
@php(list($first, $second) = [1, 2])
{{ $first + $second }}
