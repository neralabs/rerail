<x-alert type="error" :message="$message" :level="'high'" class="mt-4 {{ $extra }}" />
<x-card :title="strtoupper($title)" :$footer :count="3">
    <x-slot:header>{{ $header }}</x-slot:header>
    {{ $slot }}
</x-card>
<x-forms.input name="email" ::bind="notPhp" :value="old('email', $default)" required />
<x-icon :size="1 + 1" />
