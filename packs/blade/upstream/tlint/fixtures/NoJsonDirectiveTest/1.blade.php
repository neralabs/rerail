@extends('layouts.app')

@section('content')
    <div>
        <test :files='@json([['name' => "Logan's thing"], ["name" => "ok"]])"></test>
    </div>
@endsection