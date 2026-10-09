@csrf
@method('PUT')
<input @checked($active) @disabled(false) @readonly($locked) @required(true)>
<option @selected($choice === 'a')>a</option>
<div @class(['p-4', 'font-bold' => $bold])></div>
<div @style(['color: red' => $red])></div>
@lang('messages.welcome')
@choice('messages.apples', 10)
@auth('admin') admin @endauth
@guest guest @endguest
@can('update', $post) can @elsecan('view', $post) view @endcan
@env('local') local @endenv
@production prod @endproduction
@once <p>{{ $onceOnly }}</p> @endonce
@push('scripts') <script>{{ $pushed }}</script> @endpush
@stack('scripts')
@dd($debug)
@unset($gone)
{{ $gone }}
