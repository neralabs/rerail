@error('email')
    <p>{{ $message }}</p>
@enderror
{{ $message }}
@error('name', 'login')
    {{ $message }}
@else
    fine
@enderror
@session('status')
    {{ $value }}
@endsession
{{ $value }}
@context('trace')
    {{ $value }}
@endcontext
