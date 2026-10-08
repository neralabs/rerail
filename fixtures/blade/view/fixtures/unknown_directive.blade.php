@custom($value)
@datetime($date)
{{ $shown }}
@guarded(exec('x'))
    {{ $inside }}
@endguarded
@unlessbeta(system('y'))
    {{ $beta }}
@endbeta
@if ($shownFlag)
    shown
@elseguarded(shell('z'))
    {{ $elsewhere }}
@endif
