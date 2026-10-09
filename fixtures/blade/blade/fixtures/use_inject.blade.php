@use('App\Models\User')
@use('App\Support\Str', 'S')
@use(App\Models\Post, 'P')
@inject('metrics', 'App\Services\Metrics')
{{ $metrics->count() }} {{ User::class }} {{ S::slug($title) }} {{ P::query() }}
@inject('clock', App\Clock::class)
{{ $clock->now() }}
