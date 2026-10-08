@php
    $limit = 3;
@endphp
<script nonce="{{ $nonce }}" src="{{ asset('app.js') }}"></script>
<script type="text/x-template" id="v-form-template">
    <x-form :action="route('save')" method="POST">
        @if ($user->admin)
            <p>@lang('form.admin', ['name' => $user->name])</p>
        @endif
        @foreach ($fields as $field)
            <x-input :name="$field->name" :value="old($field->name)" />
        @endforeach
        @php
            $shown = $limit;
        @endphp
        <span>{{ $shown }}</span>
    </x-form>
</script>
<script>
    const rules = [{ operator: '<=', label: "@lang('rules.less')" }, { operator: '>', label: "@lang('rules.more')" }];
    @if ($debug)
        console.log({{ $limit }});
    @endif
</script>
