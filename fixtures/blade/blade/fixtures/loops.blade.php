@foreach ($users as $user)
    {{ $loop->index }}: {{ $user->name }}
    @if ($loop->first) first @endif
    @continue($user->skip)
    @break($loop->index > 5)
@endforeach
{{ $loop }}
@forelse ($posts as $key => $post)
    {{ $key }} {{ $post }} {{ $loop->iteration }}
@empty
    no posts
@endforelse
@foreach ($rows as $row)
    @for ($i = 0; $i < 3; $i++)
        {{ $i }} {{ $row }}
        @break(2)
    @endfor
@endforeach
@while ($queue->valid())
    {{ $queue->current() }}
    @php $queue->next(); @endphp
@endwhile
