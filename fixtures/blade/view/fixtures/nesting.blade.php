@section('title', __('Unauthorized'))
@section('code', strtoupper(trim(' 401 ')))
<head>
    <title>@yield('title', config('app.name'))</title>
</head>
<ul class="pagination">
    @if ($paginator->hasMorePages())
        <li><a href="{{ $paginator->nextPageUrl() }}">@lang('pagination.next')</a></li>
    @else
        <li class="disabled">@lang('pagination.next')</li>
    @endif
    @foreach ($elements as $element)
        <li @class(['active' => $element === ($current ?? null)])>{{ $element }}</li>
    @endforeach
</ul>
