<h1>{{ $title }}</h1>
<p>{!! $html !!}</p>
<p>{{ $user->name ?? 'guest' }} and {{ strtoupper($user->email) }}</p>
<p>@{{ $notCompiled }} and {{ 'literal' }} and {{ 1 + 2 * 3 }}</p>
<p>{!! '<b>' . 'bold' . '</b>' !!} {{ [1, 2][0] }} {{ true ? 'yes' : 'no' }}</p>
<span>{{ $a ?: $b }}</span>
{{ "double $title" }}
{{ PHP_INT_MAX }} {{ 0x1F }} {{ 1.5e3 }} {{ null ?? 'fallback' }}
