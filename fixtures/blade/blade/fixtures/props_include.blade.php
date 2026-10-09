@props(['type' => 'info', 'message'])
<div {{ $attributes->merge(['class' => 'alert-' . $type]) }}>{{ $message }}</div>
{{ $undeclared }}
@include('partials.header', ['title' => $title])
@includeIf('partials.optional')
@includeWhen($show, 'partials.when', ['x' => 1])
@each('partials.item', $items, 'item', 'partials.none')
@extends('layouts.app')
@section('title', 'Home')
@section('content')
    {{ $body }}
@endsection
@yield('sidebar', 'default')
