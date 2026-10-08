@if(true)
    This is true.
@elseif(false)
    This is false.
@endif

@if(true) @if($inline) Inline @endif @endif

@unless(true)
    This isn't true.
@endunless

@for($i = 0; $i < 10; $i++)
    The current value is {{ $i }}
@endfor

@foreach($users as $user)
    @foreach($user->emails as $email)
        <li>{{ $email }}</li>
    @endforeach
@endforeach

@forelse($users as $user)
    <li>{{ $user->name }}</li>
@empty
    <p>No users</p>
@endforelse

@while(true)
    <p>I'm looping forever.</p>
@endwhile