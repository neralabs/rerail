@foreach([
    Laravel\Memberships\Membership::MEMBER_ROLE,
    Laravel\Memberships\Membership::SUPERVISION_ROLE,
    Laravel\Memberships\Membership::ADMIN_ROLE,
    ] as $role)
    <option value="{{ $role }}">{{ $role }}</option>
@endforeach