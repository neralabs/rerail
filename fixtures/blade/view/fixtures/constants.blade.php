{{ 'a' . 'b' }} {{ 10 / 4 }} {{ 7 % 3 }} {{ 2 ** 10 }} {{ -5 }} {{ !true }}
{{ [1, 2, 3] }} {{ ['k' => 'v', 2 => 3] }} {{ 1 <=> 2 }} {{ 'x' == 0 }} {{ '1e3' == '1000' }}
{{ (int) '42abc' }} {{ (string) 1.0 }} {{ (bool) '0' }} {{ 0.1 + 0.2 }} {{ PHP_EOL }}
{!! "tab\there" !!} {!! 'it\'s' !!} {{ 0b101 | 0o7 }} {{ 1_000_000 }} {{ "a{$x}" }}
@if (2 > 1) yes @endif
@unless ('') empty @endunless
@switch('b')
    @case('a') a @break
    @case('b') b @break
@endswitch
<x-badge :count="2 * 21" :label="'n' . 1" :flags="[true, null]" />
<?= 3 * 3 ?>
@checked(1 == '1')
