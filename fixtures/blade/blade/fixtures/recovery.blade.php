@if ($list->hasPages())
    <nav>
        <ul>
            @if ($list->onFirstPage())
                <li><span>&lsaquo;</span></li>
            @else
                <li><a href="{{ $list->previousPageUrl() }}">&lsaquo;</a></li>
            @endif

            @foreach ($groups as $group)
                @if (is_string($group))
                    <li><span>{{ $group }}</span></li>
                @endif

                @if (is_array($group))
                    @foreach ($group as $number => $link)
                        @if ($number == $list->currentPage())
                            <li class="active"><span>{{ $number }}</span></li>
                        @else
                            <li><a href="{{ $link }}">{{ $number }}</a></li>
                        @endif
                    @endforeach
                @endif
            @endforeach

            {{-- the next link --}}
            @if ($list->hasMorePages())
                <li><a href="{{ $list->nextPageUrl() }}" aria-label="@lang('pagination.next')">&rsaquo;</a></li>
            @else
                <li class="disabled"><span>&rsaquo;</span></li>
            @endif
        </ul>
    </nav>
@endif
