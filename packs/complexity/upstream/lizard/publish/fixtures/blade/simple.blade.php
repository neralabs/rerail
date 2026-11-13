<?php
// Only simple functions, beside template directives that decide a lot but run in no function: nothing here is over the
// limit.

function initials(string $name): string
{
    return strtoupper(substr($name, 0, 1));
}

function label(int $count): string
{
    return $count === 1 ? 'item' : 'items';
}
?>
@foreach ($users as $user)
    @if ($user->admin && $user->active)
        <b>{{ initials($user->name) }}</b>
    @elseif ($user->active || $user->invited)
        <i>{{ initials($user->name) }}</i>
    @elseif ($user->suspended)
        <s>{{ $user->name }}</s>
    @else
        <span>{{ $user->name ?? 'anonymous' }}</span>
    @endif
    @switch ($user->role)
        @case('owner')
            <em>owner</em>
            @break
        @default
            <em>{{ $user->role }}</em>
    @endswitch
@endforeach
<p>{{ count($users) }} {{ label(count($users)) }}</p>
