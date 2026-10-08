@if ($user->isAdmin())
    admin
@elseif ($user->isEditor() && true)
    editor
@else
    {{ $fallback }}
@endif
@unless ($hidden)
    shown
@endunless
@isset($records)
    {{ count($records) }}
@endisset
@empty($items)
    nothing
@endempty
@if (1 === 1)
    always
@endif
@switch($kind)
    @case(1)
        one
        @break
    @case('two')
        two
        @break
    @default
        other
@endswitch
@if ($ok /* ) */ && $other)
    commented
@endif
@if ($first // )
    && $second)
    lined
@endif
@if ($hash # )
    || $tag)
    hashed
@endif
@if ((<<<'EOT'
    )
    EOT) !== $nowdoc)
    nowdoc
@endif
