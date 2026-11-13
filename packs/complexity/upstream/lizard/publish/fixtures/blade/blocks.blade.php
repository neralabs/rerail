{{-- Functions in @php blocks and closures in echoes, which Blade compiles to PHP and lizard reads as template text:
     lizard lists none of them, Rerail scores each (listed deviations). --}}
@php
    $grade = function (int $score, bool $late) {
        if ($score > 90 && !$late) {
            return 'A';
        }
        if ($score > 80 || $late) {
            return 'B';
        }
        if ($score > 70) {
            return 'C';
        }
        return $score > 60 ? 'D' : 'F';
    };
@endphp
<ul>
    @foreach ($items as $item)
        <li>{{ $grade($item->score, $item->late) }}</li>
    @endforeach
</ul>
<p>{{ collect($items)->filter(function ($item) { if ($item->hidden) { return false; } return $item->score > 0; })->count() }}</p>
<p>{{ collect($items)->map(fn ($item) => $item->late ? 'late' : 'on time')->implode(', ') }}</p>
