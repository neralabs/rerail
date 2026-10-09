<div>@icon('download')</div>
<button refs="dropdown@toggle" class="toggle">@icon('caret-down')</button>
<a refs="list@link-type-{{ $kind }}" href="{{ $url }}">{{ $label }}</a>
<input
    wire:model.live.debounce.{{ $debounce }}="search"
    x-on:{{ $eventName }}.window="if ($event.detail.id === @js($id)) close()"
    value="{{ $value }}"
/>
<!-- {{ $commented }} -->
@capture($content, $logo)
    <img src="{{ $logo }}" alt="{{ $brand }}">
@endcapture
@foreach ($sections as $section)
    @php ($data = $section->options) @endphp
    {{ $data }}
@endforeach
<x-control
    :value="$first"
    :value="old('type')"
    title="{{ $title }}"
/>
