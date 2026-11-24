import jwt

from app.models import Users


async def exchange_token(client, form_data):
    # The provider's userinfo endpoint accepts only a token it issued.
    token_data = {"access_token": form_data.token, "token_type": "Bearer"}
    user_data = await client.userinfo(token=token_data)
    claims = jwt.decode(form_data.token, options={"verify_signature": False})
    return await Users.update_user_roles(user_data["email"], claims.get("roles", []))


async def exchange_token_unchecked(client, form_data):
    claims = jwt.decode(form_data.token, options={"verify_signature": False})  # caught
    return await Users.update_user_roles(claims["email"], claims.get("roles", []))
